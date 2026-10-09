import { useEffect, useRef } from "react";
import { AiModelsPane, SpeechModelsPane } from "./settings/engines/EnginesPane";

/** 语音模型 as a page of the main layout (user feedback 2026-09-28: every sidebar entry but 设置 is
 *  a page, not a dialog): the recognition providers, the local model library and the recognition
 *  options. A form-like page, so it keeps a readable measure instead of the table pages' 1440 px. */
export function SpeechModels() {
  return (
    <div className="mx-auto flex w-full max-w-[1100px] flex-col p-6" data-testid="page-speech">
      <SpeechModelsPane />
    </div>
  );
}

/** AI 模型 as a page: whether the clean-up runs, its presets, and the LLM providers behind it and
 *  voice edit. `section="presets"` (the preset menus' 管理预设…) scrolls the 预设 section into view. */
export function AiModels({ section }: { section?: "presets" }) {
  const page = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (section !== "presets") return;
    const target = page.current?.querySelector('[data-testid="presets-section"]');
    target?.scrollIntoView?.({ block: "start" });
  }, [section]);
  return (
    <div
      ref={page}
      className="mx-auto flex w-full max-w-[1100px] flex-col p-6"
      data-testid="page-ai">
      <AiModelsPane />
    </div>
  );
}
