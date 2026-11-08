# GNOME 50 on Wayland without a screen, for scripts/repro-gnome-remote-desktop.sh
# (docs/dictation.md §14.9, issue #235): headless GNOME Shell with Xwayland, the remote-desktop
# portal and its GNOME backend, the accessibility bus that answers the portal's window, and a GTK
# text field that receives the pastes. Ubuntu packages only.
FROM ubuntu:26.04
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8
# Some 600 packages: the downloads stay in BuildKit's cache, so a build that is interrupted (or the
# next one) does not fetch them again.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
 && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-downloads \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      gnome-shell xwayland \
      xdg-desktop-portal xdg-desktop-portal-gnome \
      dbus dbus-bin dbus-daemon at-spi2-core libglib2.0-bin \
      python3 python3-gi gir1.2-atspi-2.0 gir1.2-gtk-4.0 \
      libgl1-mesa-dri libegl-mesa0 mesa-libgallium \
      gsettings-desktop-schemas xkb-data fonts-dejavu-core procps
# Where Mutter puts the Xwayland sockets, as on a desktop system: root's, world-writable, sticky.
# No logind here: without its seat directory GNOME Shell takes the session as one without systemd.
RUN mkdir -m 1777 /tmp/.X11-unix && rm -rf /run/systemd/seats /run/systemd/sessions /run/systemd/machines
# The image's own user, uid 1000.
USER ubuntu
WORKDIR /home/ubuntu
