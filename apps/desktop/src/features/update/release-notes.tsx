import type { ReactNode } from "react";

/** One block of release notes as release-please writes them. */
export type NotesBlock =
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; items: string[] }
  | { kind: "paragraph"; text: string };

/** One run of inline text. */
export interface InlinePart {
  kind: "text" | "bold" | "code";
  text: string;
}

const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^\s*[*-]\s+(.*)$/;

/** The markdown of a release's notes as blocks: headings, bullet lists and paragraphs. Anything
 *  else stays text. A leading heading that repeats `version` is dropped: the dialog's title says
 *  it already. */
export function parseNotes(markdown: string, version?: string): NotesBlock[] {
  const blocks: NotesBlock[] = [];
  let paragraph: string[] = [];
  let list: string[] | undefined;
  const flushParagraph = () => {
    if (paragraph.length > 0) blocks.push({ kind: "paragraph", text: paragraph.join(" ") });
    paragraph = [];
  };
  const flushList = () => {
    if (list !== undefined) blocks.push({ kind: "list", items: list });
    list = undefined;
  };
  for (const raw of markdown.replaceAll("\r\n", "\n").split("\n")) {
    const line = raw.trimEnd();
    const heading = HEADING.exec(line.trim());
    const bullet = BULLET.exec(line);
    if (heading?.[1] !== undefined && heading[2] !== undefined) {
      flushParagraph();
      flushList();
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2].trim() });
    } else if (bullet?.[1] !== undefined) {
      flushParagraph();
      (list ??= []).push(bullet[1].trim());
    } else if (line.trim().length === 0) {
      flushParagraph();
      flushList();
    } else {
      flushList();
      paragraph.push(line.trim());
    }
  }
  flushParagraph();
  flushList();
  const first = blocks[0];
  if (
    version !== undefined &&
    first?.kind === "heading" &&
    first.text.replace(/^\[|\]$|^v/gu, "").startsWith(version)
  )
    blocks.shift();
  return blocks;
}

/** Inline markup: `**bold**`, `` `code` ``, and `[text](url)` as its text (the webview opens
 *  no link of its own). Parentheses left empty by a dropped commit link go too. */
export function inlineParts(text: string): InlinePart[] {
  const plain = text
    // A URL may hold one level of parentheses of its own (`…(1)`).
    .replaceAll(/\[([^\]]*)\]\((?:[^()]|\([^()]*\))*\)/gu, "$1")
    .replaceAll(/\s*\(\s*\)/gu, "");
  const parts: InlinePart[] = [];
  const pattern = /\*\*([^*]+)\*\*|`([^`]+)`/gu;
  let last = 0;
  for (const match of plain.matchAll(pattern)) {
    if (match.index > last) parts.push({ kind: "text", text: plain.slice(last, match.index) });
    if (match[1] !== undefined) parts.push({ kind: "bold", text: match[1] });
    else if (match[2] !== undefined) parts.push({ kind: "code", text: match[2] });
    last = match.index + match[0].length;
  }
  if (last < plain.length) parts.push({ kind: "text", text: plain.slice(last) });
  return parts;
}

function Inline({ text }: { text: string }): ReactNode {
  return inlineParts(text).map((part, i) =>
    part.kind === "bold" ? (
      <strong key={i} className="font-semibold text-fg">
        {part.text}
      </strong>
    ) : part.kind === "code" ? (
      <code key={i} className="mono rounded-6 bg-inset px-1 text-[12px]">
        {part.text}
      </code>
    ) : (
      <span key={i}>{part.text}</span>
    ),
  );
}

/** Release notes drawn as React elements. Nothing is interpreted as HTML. */
export function ReleaseNotes({ markdown, version }: { markdown: string; version?: string }) {
  const blocks = parseNotes(markdown, version);
  return (
    <div
      className="flex flex-col gap-2 text-[13px] leading-5 text-fg-muted"
      data-testid="release-notes">
      {blocks.map((block, i) => {
        if (block.kind === "heading")
          return (
            <h4
              key={i}
              className={
                block.level <= 2
                  ? "mt-1 text-[14px] font-semibold text-fg"
                  : "mt-1 text-[13px] font-semibold text-fg"
              }>
              <Inline text={block.text} />
            </h4>
          );
        if (block.kind === "list")
          return (
            <ul key={i} className="flex list-disc flex-col gap-1 pl-5">
              {block.items.map((item, j) => (
                <li key={j}>
                  <Inline text={item} />
                </li>
              ))}
            </ul>
          );
        return (
          <p key={i}>
            <Inline text={block.text} />
          </p>
        );
      })}
    </div>
  );
}
