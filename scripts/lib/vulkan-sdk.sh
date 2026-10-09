# shellcheck shell=bash
# Vulkan for the packaged Windows and Linux builds (docs/dictation.md §10.6). Sourced by the
# packaging scripts and the release workflow; everything is pinned by version and SHA-256 and kept in
# ${VOLTIP_VULKAN_CACHE:-$HOME/.cache/voltip-vulkan}:
#   - the LunarG Vulkan SDK for Linux x86_64: headers, glslc, SPIRV-Headers and the loader to link
#     against. Both packages use it: the Windows one is cross-built on Linux;
#   - LunarG's Vulkan runtime components for Windows x64: the Khronos loader `vulkan-1.dll` the
#     Windows package ships next to the exe, and an import library generated from its exports.
# The loader is linked dynamically: `vulkan-1.dll` (shipped) on Windows, `libvulkan.so.1` on Linux (a
# package dependency; the AppImage carries its own). With no Vulkan driver installed the loader
# reports no device and the local models run on the CPU.

VOLTIP_VULKAN_SDK_VERSION=1.4.357.1
VOLTIP_VULKAN_SDK_SHA256=4b41e3b30e8aedaa5dac7c136561ab463eb316a25a54e2c6245f2c299ea1fb85
VOLTIP_VULKAN_RUNTIME_VERSION=1.4.357.0
VOLTIP_VULKAN_RUNTIME_SHA256=a14672efed15aafc7f5a16572d35cd3a3416eadf670aeee3cdf50ee32d5fbf83

voltip_vulkan_cache() {
  printf '%s\n' "${VOLTIP_VULKAN_CACHE:-$HOME/.cache/voltip-vulkan}"
}

# Print the cached path of <file>, downloading it from <url> first unless it is there with <sha256>.
voltip_vulkan_fetch() { # <url> <file> <sha256>
  local cache file
  cache=$(voltip_vulkan_cache)
  file=$cache/$2
  mkdir -p "$cache"
  if [ -f "$file" ] && echo "$3  $file" | sha256sum -c --status; then
    printf '%s\n' "$file"
    return 0
  fi
  rm -f "$file" "$file.part"
  echo "vulkan: downloading $1" >&2
  curl -sSfL --retry 3 -o "$file.part" "$1" || { echo "vulkan: download failed: $1" >&2; return 1; }
  if ! echo "$3  $file.part" | sha256sum -c --status; then
    echo "vulkan: $2 does not match its pinned SHA-256" >&2
    rm -f "$file.part"
    return 1
  fi
  mv "$file.part" "$file"
  printf '%s\n' "$file"
}

# Print the SDK's x86_64 root: include/, bin/glslc, lib/VulkanLoader/lib/, share/cmake/SPIRV-Headers/.
voltip_vulkan_sdk() {
  local cache root tarball
  cache=$(voltip_vulkan_cache)
  root=$cache/$VOLTIP_VULKAN_SDK_VERSION/x86_64
  if [ ! -x "$root/bin/glslc" ]; then
    tarball=$(voltip_vulkan_fetch \
      "https://sdk.lunarg.com/sdk/download/$VOLTIP_VULKAN_SDK_VERSION/linux/vulkansdk-linux-x86_64-$VOLTIP_VULKAN_SDK_VERSION.tar.xz" \
      "vulkansdk-linux-x86_64-$VOLTIP_VULKAN_SDK_VERSION.tar.xz" "$VOLTIP_VULKAN_SDK_SHA256") || return 1
    rm -rf "${cache:?}/$VOLTIP_VULKAN_SDK_VERSION"
    tar -xf "$tarball" -C "$cache" || return 1
    [ -x "$root/bin/glslc" ] || { echo "vulkan: $tarball has no x86_64/bin/glslc" >&2; return 1; }
  fi
  printf '%s\n' "$root"
}

# Print a directory holding the Windows x64 loader `vulkan-1.dll`, its licence and `vulkan-1.lib`,
# the import library generated from the DLL's own export table (llvm-readobj + llvm-dlltool).
voltip_vulkan_windows_runtime() {
  local cache dir zip base
  cache=$(voltip_vulkan_cache)
  dir=$cache/runtime-windows-x64-$VOLTIP_VULKAN_RUNTIME_VERSION
  if [ ! -f "$dir/vulkan-1.lib" ]; then
    for tool in unzip llvm-readobj llvm-dlltool; do
      command -v "$tool" >/dev/null || { echo "vulkan: $tool not installed" >&2; return 1; }
    done
    base=VulkanRT-X64-$VOLTIP_VULKAN_RUNTIME_VERSION-Components
    zip=$(voltip_vulkan_fetch \
      "https://sdk.lunarg.com/sdk/download/$VOLTIP_VULKAN_RUNTIME_VERSION/windows/$base.zip" \
      "$base.zip" "$VOLTIP_VULKAN_RUNTIME_SHA256") || return 1
    rm -rf "$dir.tmp"
    mkdir -p "$dir.tmp"
    unzip -q -j "$zip" "$base/x64/vulkan-1.dll" "$base/VulkanRT-License.txt" -d "$dir.tmp" || return 1
    {
      echo "LIBRARY vulkan-1.dll"
      echo "EXPORTS"
      llvm-readobj --coff-exports "$dir.tmp/vulkan-1.dll" | sed -n 's/^ *Name: \(vk[A-Za-z0-9_]*\)$/    \1/p'
    } >"$dir.tmp/vulkan-1.def"
    grep -q "vkCreateInstance$" "$dir.tmp/vulkan-1.def" || { echo "vulkan: no exports read from vulkan-1.dll" >&2; return 1; }
    llvm-dlltool -m i386:x86-64 -d "$dir.tmp/vulkan-1.def" -l "$dir.tmp/vulkan-1.lib" || return 1
    rm -rf "$dir"
    mv "$dir.tmp" "$dir"
  fi
  printf '%s\n' "$dir"
}

# Print TRANSCRIBE_CMAKE_ARGS for a Vulkan build: the CPU baseline every build uses
# (.cargo/config.toml) plus the SDK paths, given explicitly so a cross toolchain's find-root rules
# cannot hide them. <loader> is the library to link: libvulkan.so or the generated vulkan-1.lib.
voltip_vulkan_cmake_args() { # <sdk-root> <loader>
  local baseline
  baseline=$(sed -n 's/^TRANSCRIBE_CMAKE_ARGS = "\(.*\)"$/\1/p' .cargo/config.toml)
  [ -n "$baseline" ] || { echo "vulkan: TRANSCRIBE_CMAKE_ARGS not found in .cargo/config.toml" >&2; return 1; }
  printf '%s\n' "$baseline -DVulkan_INCLUDE_DIR=$1/include -DVulkan_LIBRARY=$2 -DVulkan_GLSLC_EXECUTABLE=$1/bin/glslc -DSPIRV-Headers_DIR=$1/share/cmake/SPIRV-Headers"
}

# Export what a Vulkan build of the desktop needs for <target> (`linux` or `windows`) and stage the
# loader the package ships into apps/desktop/src-tauri/resources/<target>/ (git-ignored):
# `vulkan-1.dll` (+ its licence) for Windows, `libvulkan.so.1` for the AppImage on Linux.
voltip_vulkan_env() { # <linux|windows>
  local sdk runtime resources=apps/desktop/src-tauri/resources/$1
  sdk=$(voltip_vulkan_sdk) || return 1
  export VULKAN_SDK=$sdk
  mkdir -p "$resources"
  case $1 in
    linux)
      TRANSCRIBE_CMAKE_ARGS=$(voltip_vulkan_cmake_args "$sdk" "$sdk/lib/VulkanLoader/lib/libvulkan.so") || return 1
      # The link uses the SDK's loader; running the binary here takes the system's libvulkan.so.1.
      export VOLTIP_VULKAN_LIB_DIR=$sdk/lib/VulkanLoader/lib
      cp -L "$sdk/lib/VulkanLoader/lib/libvulkan.so.1" "$resources/libvulkan.so.1"
      ;;
    windows)
      runtime=$(voltip_vulkan_windows_runtime) || return 1
      TRANSCRIBE_CMAKE_ARGS=$(voltip_vulkan_cmake_args "$sdk" "$runtime/vulkan-1.lib") || return 1
      export VOLTIP_VULKAN_LIB_DIR=$runtime
      cp "$runtime/vulkan-1.dll" "$resources/vulkan-1.dll"
      cp "$runtime/VulkanRT-License.txt" "$resources/vulkan-1-LICENSE.txt"
      ;;
    *)
      echo "vulkan: unknown target $1 (linux or windows)" >&2
      return 1
      ;;
  esac
  export TRANSCRIBE_CMAKE_ARGS
}
