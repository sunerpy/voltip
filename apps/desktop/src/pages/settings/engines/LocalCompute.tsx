import {
  type EngineSettings,
  type GpuDevice,
  type LocalDevice,
  MAX_LOCAL_THREADS,
  type TFunction,
} from "@voltip/shared";
import {
  Segmented,
  Select,
  SettingsRows,
  SettingsSection,
  StatusRow,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";

/** `local_threads` unset: each engine picks its own. */
const AUTO_THREADS = "auto";

/** Thread counts the select offers: powers of two up to the machine, plus the machine itself,
 *  never above what the core accepts (`MAX_LOCAL_THREADS`). */
export function threadChoices(cpuThreads: number): number[] {
  const top = Math.min(cpuThreads, MAX_LOCAL_THREADS);
  const out: number[] = [];
  for (let n = 1; n < top; n *= 2) out.push(n);
  if (top > 0) out.push(top);
  return out;
}

/** `NVIDIA L40S · 45 GB` / `Intel UHD Graphics 770 · 集成显卡`. */
export function gpuLabel(gpu: GpuDevice, t: TFunction): string {
  const detail = gpu.integrated
    ? t("engines.compute.integrated")
    : gpu.memory_mb > 0
      ? t("engines.compute.memory", { gb: Math.round(gpu.memory_mb / 1024) })
      : gpu.kind;
  return `${gpu.description.length > 0 ? gpu.description : gpu.name} · ${detail}`;
}

/** Settings › 引擎 › 本机识别 › 运行设备 (docs/dictation.md §10.6): auto / CPU / GPU, which GPU,
 *  and the inference threads, written through `settings_set_engines`. The GPU choice is only
 *  offered for GPUs the desktop reports this build can drive (`UiState.hardware`); without one the
 *  models run on the CPU and the row says so. */
export function LocalCompute() {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { settings, hardware } = useUiState();
  const engines = settings.engines;
  const gpus = hardware.gpus;
  const write = (patch: Partial<EngineSettings>) => {
    void backend.invoke("settings_set_engines", { engines: { ...engines, ...patch } });
  };
  const device: LocalDevice = engines.local_device;
  const gpu = gpus.find((g) => g.name === engines.local_gpu) ?? gpus[0];
  const threads = engines.local_threads ?? undefined;
  const choices = threadChoices(hardware.cpu_threads);
  // A saved count the machine does not offer (another computer's settings) stays selectable.
  if (threads !== undefined && !choices.includes(threads)) choices.push(threads);
  return (
    <SettingsSection
      title={t("engines.compute.title")}
      description={t("engines.compute.description")}
      data-testid="local-compute"
      data={{ "data-device": device }}>
      <SettingsRows>
        <StatusRow
          label={t("engines.compute.device")}
          help={gpus.length === 0 ? t("engines.compute.noGpu") : t("engines.compute.autoHelp")}>
          <Segmented
            label={t("engines.compute.device")}
            value={device}
            onChange={(next) => {
              write(
                next === "gpu" && gpu !== undefined
                  ? { local_device: next, local_gpu: gpu.name }
                  : { local_device: next },
              );
            }}
            options={[
              { value: "auto", label: t("engines.compute.auto") },
              { value: "cpu", label: t("engines.compute.cpu") },
              { value: "gpu", label: t("engines.compute.gpu"), disabled: gpus.length === 0 },
            ]}
          />
        </StatusRow>
        {device === "gpu" && gpu !== undefined && (
          <StatusRow label={t("engines.compute.whichGpu")} help={t("engines.compute.gpuFirstLoad")}>
            <Select
              aria-label={t("engines.compute.whichGpu")}
              value={gpu.name}
              onChange={(name) => {
                write({ local_gpu: name });
              }}
              options={gpus.map((g) => ({ value: g.name, label: gpuLabel(g, t) }))}
            />
          </StatusRow>
        )}
        <StatusRow
          label={t("engines.compute.threads")}
          help={
            hardware.cpu_threads > 0
              ? t("engines.compute.threadsHelp", { n: hardware.cpu_threads })
              : undefined
          }>
          <Select
            aria-label={t("engines.compute.threads")}
            value={threads === undefined ? AUTO_THREADS : String(threads)}
            onChange={(value) => {
              write({ local_threads: value === AUTO_THREADS ? null : Number(value) });
            }}
            options={[
              { value: AUTO_THREADS, label: t("engines.compute.threadsAuto") },
              ...choices.map((n) => ({ value: String(n), label: String(n) })),
            ]}
          />
        </StatusRow>
      </SettingsRows>
    </SettingsSection>
  );
}
