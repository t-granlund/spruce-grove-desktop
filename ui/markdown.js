/* ============================================================================
   markdown.js — a small, SAFE markdown renderer for the live stream.

   Why hand-rolled: this app keeps ONE rule everywhere — agent text is never
   turned into HTML. No `innerHTML`, no `insertAdjacentHTML`, no template
   strings fed to the DOM. The CSP even drops `unsafe-inline` for scripts. A
   markdown-to-HTML library would break that rule and reopen the exact XSS
   class the codebase is proud of closing.

   So this renders by BUILDING NODES: every piece of text goes through
   `textContent`, every link through a real element + listener. A message like
   `<img src=x onerror=alert(1)>` becomes literal, inert text — which is the
   point.

   Scope: the subset the CLI actually emits — headings, paragraphs, bullet and
   numbered lists, fenced code, inline code, bold, italic, strikethrough,
   blockquotes, horizontal rules, and links. Unknown syntax falls through as
   plain text rather than guessing.

   Streaming: `render()` is idempotent and cheap, so the caller can re-render the
   whole accumulated message on each chunk (throttled). See `streamMarkdown`.
============================================================================ */
(function () {
  "use strict";

  function el(tag, cls) {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    return n;
  }
  function isBlank(s) { return !s || !s.trim(); }

  /* ------------------------------ inline -------------------------------- */

  // A scanning inline parser. Order matters: code spans win, so `**` inside
  // backticks stays literal. Every branch emits nodes with textContent.
  const INLINE = /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*]+\*)|(_[^_]+_)|(~~[^~]+~~)|(\[[^\]]+\]\([^)]+\))/;

  function inline(text, parent) {
    let rest = text;
    while (rest.length) {
      const m = INLINE.exec(rest);
      if (!m) { parent.appendChild(document.createTextNode(rest)); break; }
      if (m.index > 0) parent.appendChild(document.createTextNode(rest.slice(0, m.index)));
      const tok = m[0];
      let node;
      if (tok.startsWith("`")) {
        node = el("code", "md-code");
        node.textContent = tok.slice(1, -1);
      } else if (tok.startsWith("**") || tok.startsWith("__")) {
        node = el("strong");
        inline(tok.slice(2, -2), node);
      } else if (tok.startsWith("~~")) {
        node = el("del");
        inline(tok.slice(2, -2), node);
      } else if (tok.startsWith("*") || tok.startsWith("_")) {
        node = el("em");
        inline(tok.slice(1, -1), node);
      } else {
        // [label](url)
        const mm = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(tok);
        node = el("a", "md-link");
        node.textContent = mm[1];
        node.href = mm[2];
        node.title = mm[2];
        if (/^https?:\/\//.test(mm[2])) {
          // open in the real browser, never inside the webview
          node.addEventListener("click", function (ev) {
            ev.preventDefault();
            try { invoke("grove_open_url", { url: mm[2] }); } catch (_) {}
          });
        } else {
          node.removeAttribute("href");
        }
      }
      parent.appendChild(node);
      rest = rest.slice(m.index + tok.length);
    }
  }

  /* ------------------------------- blocks ------------------------------- */

  const FENCE = /^\s*```(.*)$/;
  const HEAD = /^(#{1,6})\s+(.*)$/;
  const UL = /^\s*[-*+]\s+(.*)$/;
  const OL = /^\s*(\d+)\.\s+(.*)$/;
  const QUOTE = /^\s*>\s?(.*)$/;
  const HR = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;

  function renderBlocks(text, host) {
    const lines = text.replace(/\r\n?/g, "\n").split("\n");
    let i = 0;
    while (i < lines.length) {
      const line = lines[i];

      if (isBlank(line)) { i++; continue; }

      // fenced code: take everything to the closing fence (or EOF mid-stream)
      const fence = FENCE.exec(line);
      if (fence) {
        i++;
        const buf = [];
        while (i < lines.length && !FENCE.test(lines[i])) { buf.push(lines[i]); i++; }
        if (i < lines.length) i++; // consume closing fence
        const pre = el("pre", "md-pre");
        const code = el("code");
        code.textContent = buf.join("\n");
        pre.appendChild(code);
        host.appendChild(pre);
        continue;
      }

      // horizontal rule
      if (HR.test(line)) { host.appendChild(el("hr", "md-hr")); i++; continue; }

      // heading
      const head = HEAD.exec(line);
      if (head) {
        const h = el("h" + head[1].length, "md-h" + head[1].length);
        inline(head[2].trim(), h);
        host.appendChild(h);
        i++;
        continue;
      }

      // blockquote (consecutive >)
      if (QUOTE.test(line)) {
        const buf = [];
        while (i < lines.length && QUOTE.test(lines[i])) { buf.push(QUOTE.exec(lines[i])[1]); i++; }
        const bq = el("blockquote", "md-quote");
        renderBlocks(buf.join("\n"), bq);
        host.appendChild(bq);
        continue;
      }

      // unordered list
      if (UL.test(line)) {
        const ul = el("ul", "md-ul");
        while (i < lines.length && UL.test(lines[i])) {
          const li = el("li");
          inline(UL.exec(lines[i])[1], li);
          ul.appendChild(li);
          i++;
        }
        host.appendChild(ul);
        continue;
      }

      // ordered list
      if (OL.test(line)) {
        const ol = el("ol", "md-ol");
        while (i < lines.length && OL.test(lines[i])) {
          const li = el("li");
          inline(OL.exec(lines[i])[2], li);
          ol.appendChild(li);
          i++;
        }
        host.appendChild(ol);
        continue;
      }

      // paragraph: gather until a blank line or the start of another block
      const para = [];
      while (
        i < lines.length &&
        !isBlank(lines[i]) &&
        !FENCE.test(lines[i]) && !HEAD.test(lines[i]) && !HR.test(lines[i]) &&
        !QUOTE.test(lines[i]) && !UL.test(lines[i]) && !OL.test(lines[i])
      ) {
        para.push(lines[i]);
        i++;
      }
      const p = el("p", "md-p");
      inline(para.join("\n"), p);
      host.appendChild(p);
    }
  }

  /* -------------------------------- API --------------------------------- */

  /** Render `text` as safe markdown into `host` (replacing its contents). */
  function render(host, text) {
    host.textContent = "";
    renderBlocks(text || "", host);
    return host;
  }

  window.BBMarkdown = { render: render };
})();
