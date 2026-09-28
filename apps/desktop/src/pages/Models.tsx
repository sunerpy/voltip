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

/** AI 模型 as a page: whether the clean-up runs, and the LLM providers behind it and voice edit. */
export function AiModels() {
  return (
    <div className="mx-auto flex w-full max-w-[1100px] flex-col p-6" data-testid="page-ai">
      <AiModelsPane />
    </div>
  );
}
