/**
 * FixSpecModal — an in-app popup reader for the FIX API guide.
 *
 * Fetches the served guide (`/fix/celnet-fix-api.md`) and renders it inside a
 * scrim+blur modal so an operator can read the message-flow / how-to-connect spec
 * without leaving the app, plus a Download button to save the raw markdown. The
 * markdown is rendered by a small, dependency-free, XSS-safe renderer (it builds
 * React nodes — never `dangerouslySetInnerHTML`); link hrefs are scheme-validated
 * and doc-relative `./x` links are rewritten to the served `/fix/x` path.
 *
 * Accessibility: `role="dialog"` + `aria-modal`, labelled title, Esc to close.
 */

import { Fragment, useEffect, useId, useState } from "react";

import { Button } from "./Button";
import { downloadText, FIX_API_GUIDE_URL } from "../lib/fixClientConfig";
import styles from "./FixSpecModal.module.css";

export interface FixSpecModalProps {
  open: boolean;
  onClose: () => void;
}

export function FixSpecModal({ open, onClose }: FixSpecModalProps): React.ReactElement | null {
  const titleId = useId();
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Lazily fetch the guide the first time the modal opens.
  useEffect(() => {
    if (!open || text !== null) return;
    let cancelled = false;
    void fetch(FIX_API_GUIDE_URL)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.text();
      })
      .then((body) => {
        if (!cancelled) setText(body);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : "failed to load the guide");
      });
    return () => {
      cancelled = true;
    };
  }, [open, text]);

  // Esc closes while open.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <h2 id={titleId} className={styles.title}>
            FIX API guide
          </h2>
          <div className={styles.headActions}>
            <Button
              variant="secondary"
              onClick={() => text && downloadText("celnet-fix-api.md", text, "text/markdown")}
              disabled={text === null}
            >
              Download
            </Button>
            <Button variant="ghost" onClick={onClose}>
              Close
            </Button>
          </div>
        </div>
        <div className={styles.body}>
          {error && <p className={styles.status}>Could not load the guide: {error}</p>}
          {!error && text === null && <p className={styles.status}>Loading…</p>}
          {text !== null && renderMarkdown(text)}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// A tiny, safe markdown renderer (block + inline) — builds React nodes only.
// Handles the constructs the guide uses: ATX headings, fenced code, tables,
// unordered lists, blockquotes, horizontal rules, paragraphs, and the inline
// **bold** / `code` / [text](href) spans.
// ---------------------------------------------------------------------------

/** Split a markdown table row `| a | b |` into trimmed cells. */
function splitRow(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

/** Whether a line is a table separator row (`|---|:--:|`). */
function isTableSeparator(line: string): boolean {
  return /^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(line) && line.includes("-");
}

/** Resolve/validate a link href; doc-relative `./x` → the served `/fix/x`. */
function safeHref(href: string): string | undefined {
  if (href.startsWith("./")) return `/fix/${href.slice(2)}`;
  if (href.startsWith("#") || href.startsWith("/")) return href;
  try {
    const u = new URL(href);
    if (["http:", "https:", "mailto:"].includes(u.protocol)) return href;
  } catch {
    return undefined;
  }
  return undefined;
}

const INLINE_RE = /(\*\*([^*]+)\*\*)|(`([^`]+)`)|(\[([^\]]+)\]\(([^)]+)\))/g;

/** Render inline markdown (bold / code / links) within a line. */
function renderInline(text: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let last = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  INLINE_RE.lastIndex = 0;
  while ((m = INLINE_RE.exec(text)) !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    if (m[2] !== undefined) {
      out.push(<strong key={key++}>{m[2]}</strong>);
    } else if (m[4] !== undefined) {
      out.push(
        <code key={key++} className={styles.inlineCode}>
          {m[4]}
        </code>,
      );
    } else if (m[6] !== undefined) {
      const href = safeHref(m[7] ?? "");
      out.push(
        href ? (
          <a
            key={key++}
            className={styles.link}
            href={href}
            target="_blank"
            rel="noopener noreferrer"
          >
            {m[6]}
          </a>
        ) : (
          <Fragment key={key++}>{m[6]}</Fragment>
        ),
      );
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Render a markdown document into a list of React block nodes. */
function renderMarkdown(md: string): React.ReactNode[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const blocks: React.ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i]!;

    // Fenced code block.
    if (line.startsWith("```")) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.startsWith("```")) {
        code.push(lines[i]!);
        i++;
      }
      i++; // consume closing fence
      blocks.push(
        <pre key={key++} className={styles.code}>
          <code>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    // ATX heading.
    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    if (h) {
      const level = h[1]!.length;
      const content = renderInline(h[2]!);
      const cls = level === 1 ? styles.h1 : level === 2 ? styles.h2 : styles.h3;
      if (level === 1) blocks.push(<h1 key={key++} className={cls}>{content}</h1>);
      else if (level === 2) blocks.push(<h2 key={key++} className={cls}>{content}</h2>);
      else blocks.push(<h3 key={key++} className={cls}>{content}</h3>);
      i++;
      continue;
    }

    // Horizontal rule.
    if (/^---+$/.test(line.trim())) {
      blocks.push(<hr key={key++} className={styles.hr} />);
      i++;
      continue;
    }

    // Table (header row + separator + body rows).
    if (line.trim().startsWith("|") && i + 1 < lines.length && isTableSeparator(lines[i + 1]!)) {
      const header = splitRow(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i]!.trim().startsWith("|")) {
        rows.push(splitRow(lines[i]!));
        i++;
      }
      blocks.push(
        <table key={key++} className={styles.table}>
          <thead>
            <tr>
              {header.map((c, ci) => (
                <th key={ci}>{renderInline(c)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                {r.map((c, ci) => (
                  <td key={ci}>{renderInline(c)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>,
      );
      continue;
    }

    // Blockquote.
    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i]!.startsWith(">")) {
        quote.push(lines[i]!.replace(/^>\s?/, ""));
        i++;
      }
      blocks.push(
        <blockquote key={key++} className={styles.quote}>
          {renderInline(quote.join(" "))}
        </blockquote>,
      );
      continue;
    }

    // Unordered list.
    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i]!)) {
        items.push(lines[i]!.replace(/^\s*[-*]\s+/, ""));
        i++;
      }
      blocks.push(
        <ul key={key++} className={styles.list}>
          {items.map((it, ii) => (
            <li key={ii}>{renderInline(it)}</li>
          ))}
        </ul>,
      );
      continue;
    }

    // Blank line.
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Paragraph (gather until a blank line or the next block construct).
    const para: string[] = [line];
    i++;
    while (i < lines.length) {
      const l = lines[i]!;
      if (
        l.trim() === "" ||
        l.startsWith("#") ||
        l.startsWith("```") ||
        l.startsWith(">") ||
        /^\s*[-*]\s+/.test(l) ||
        l.trim().startsWith("|") ||
        /^---+$/.test(l.trim())
      ) {
        break;
      }
      para.push(l);
      i++;
    }
    blocks.push(
      <p key={key++} className={styles.p}>
        {renderInline(para.join(" "))}
      </p>,
    );
  }

  return blocks;
}
