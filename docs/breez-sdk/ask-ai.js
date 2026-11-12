// Ask AI widget for the Breez SDK docs.
//
// Same-origin API route expected in Caddy:
//
//   handle_path /ask-api/* {
//       reverse_proxy 127.0.5.9:8080 {
//           flush_interval -1
//       }
//   }
//
// The widget reads /ask-api/ask/stream (Server-Sent Events) and renders the
// Markdown answer. The server is stateless: the browser keeps the transcript in
// sessionStorage and sends the last few question/answer pairs back with every
// request.
(function () {
    "use strict";

    var ENDPOINT = "/ask-api/ask/stream";
    var SESSION_KEY = "ask-ai-state-v2";

    // How many completed exchanges are sent to the API. The server also caps
    // this (MAX_HISTORY_TURNS) and reports its value in the relevance event,
    // which overrides this default.
    var CONTEXT_EXCHANGES = 3;
    // How many exchanges stay in the panel (and in sessionStorage). Older ones
    // are dropped so the tab does not grow without bound.
    var MAX_EXCHANGES = 10;

    // Soft-launch switch. Keep true while the team reviews the widget; set to
    // false when it should be visible to everyone.
    var PREVIEW_ONLY = true;

    function isEnabled() {
        if (!PREVIEW_ONLY) {
            return true;
        }
        try {
            return new URLSearchParams(window.location.search).get("ask-ai") === "preview";
        } catch (err) {
            return false;
        }
    }

    function init() {
        if (document.getElementById("ask-ai-trigger")) {
            return;
        }
        if (!window.fetch || !window.AbortController || !window.TextDecoder) {
            return;
        }
        if (!isEnabled()) {
            return;
        }

        var root = document.createElement("div");
        root.className = "ask-ai";
        root.innerHTML = [
            '<button class="ask-ai-trigger" id="ask-ai-trigger" type="button" aria-haspopup="dialog" aria-controls="ask-ai-panel" aria-expanded="false" aria-label="Ask AI about the Breez SDK">',
            '  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">',
            '    <path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3z"></path>',
            '    <path d="M5 17l.8 2.2L8 20l-2.2.8L5 23l-.8-2.2L2 20l2.2-.8L5 17z"></path>',
            '  </svg>',
            '  <span>Ask AI</span>',
            '</button>',
            '<section class="ask-ai-panel" id="ask-ai-panel" role="dialog" aria-modal="true" aria-labelledby="ask-ai-title" hidden>',
            '  <div class="ask-ai-head">',
            '    <div>',
            '      <div class="ask-ai-title" id="ask-ai-title">Ask AI <span class="ask-ai-badge" id="ask-ai-badge">Preview</span></div>',
            '      <div class="ask-ai-subtitle">Answers from the Breez SDK docs</div>',
            '      <div class="ask-ai-context" id="ask-ai-context" hidden></div>',
            '    </div>',
            '    <div class="ask-ai-head-actions">',
            '      <button class="ask-ai-icon" id="ask-ai-new" type="button" aria-label="Start a new conversation" title="New conversation">',
            '        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">',
            '          <path d="M3 12a9 9 0 1 0 3-6.7"></path>',
            '          <path d="M3 4v5h5"></path>',
            '        </svg>',
            '      </button>',
            '      <button class="ask-ai-icon" id="ask-ai-close" type="button" aria-label="Close Ask AI">&times;</button>',
            '    </div>',
            '  </div>',
            '  <div class="ask-ai-body" id="ask-ai-body">',
            '    <div class="ask-ai-empty" id="ask-ai-empty">',
            '      <p>Ask a question about the Breez SDK. Answers are generated from these docs and may be incomplete.</p>',
            '      <div class="ask-ai-examples">',
            '        <button type="button" data-ask-question="How do I create a Lightning invoice?">How do I create a Lightning invoice?</button>',
            '        <button type="button" data-ask-question="How do I enable stable balance?">How do I enable stable balance?</button>',
            '        <button type="button" data-ask-question="How do I use passkey login?">How do I use passkey login?</button>',
            '      </div>',
            '    </div>',
            '    <div class="ask-ai-transcript" id="ask-ai-transcript"></div>',
            '  </div>',
            '  <form class="ask-ai-form" id="ask-ai-form">',
            '    <textarea id="ask-ai-input" rows="2" maxlength="2000" placeholder="Ask about the Breez SDK&hellip;" aria-label="Your question"></textarea>',
            '    <div class="ask-ai-actions">',
            '      <span class="ask-ai-hint">Enter to send &middot; Shift+Enter for a new line</span>',
            '      <button type="button" class="ask-ai-stop" id="ask-ai-stop" hidden>Stop</button>',
            '      <button type="submit" class="ask-ai-send" id="ask-ai-send">Send</button>',
            '    </div>',
            '  </form>',
            '</section>'
        ].join("");
        document.body.appendChild(root);

        if (!PREVIEW_ONLY) {
            var badge = document.getElementById("ask-ai-badge");
            if (badge && badge.parentNode) {
                badge.parentNode.removeChild(badge);
            }
        }

        var trigger = document.getElementById("ask-ai-trigger");
        var panel = document.getElementById("ask-ai-panel");
        var closeButton = document.getElementById("ask-ai-close");
        var newButton = document.getElementById("ask-ai-new");
        var body = document.getElementById("ask-ai-body");
        var empty = document.getElementById("ask-ai-empty");
        var transcript = document.getElementById("ask-ai-transcript");
        var contextEl = document.getElementById("ask-ai-context");
        var form = document.getElementById("ask-ai-form");
        var input = document.getElementById("ask-ai-input");
        var sendButton = document.getElementById("ask-ai-send");
        var stopButton = document.getElementById("ask-ai-stop");

        // exchanges holds the whole visible conversation, oldest first. Each
        // item is {question, kind, answer, sources, message, stage}, where kind
        // is "pending", "answer", "note" or "error".
        var exchanges = [];
        var contextExchanges = CONTEXT_EXCHANGES;
        var controller = null;
        var streaming = false;
        var currentAnswer = "";
        var renderTimer = null;
        var live = null;
        var sawDone = false;
        var sawError = false;

        function loadState() {
            try {
                var raw = window.sessionStorage.getItem(SESSION_KEY);
                if (!raw) {
                    return null;
                }
                var state = JSON.parse(raw);
                if (!state || typeof state !== "object" || !Array.isArray(state.exchanges)) {
                    return null;
                }
                var limit = state.contextExchanges;
                return {
                    open: !!state.open,
                    contextExchanges: typeof limit === "number" && isFinite(limit) && limit >= 0
                        ? Math.floor(limit)
                        : null,
                    exchanges: state.exchanges.map(normalizeExchange).filter(Boolean)
                };
            } catch (err) {
                return null;
            }
        }

        function normalizeExchange(value) {
            if (!value || typeof value !== "object" || typeof value.question !== "string") {
                return null;
            }
            var kind = value.kind;
            if (["pending", "answer", "note", "error"].indexOf(kind) === -1) {
                kind = "answer";
            }
            return {
                question: value.question,
                kind: kind,
                answer: typeof value.answer === "string" ? value.answer : "",
                sources: Array.isArray(value.sources)
                    ? value.sources.filter(function (url) {
                        return typeof url === "string";
                    })
                    : [],
                message: typeof value.message === "string" ? value.message : "",
                stage: typeof value.stage === "string" ? value.stage : ""
            };
        }

        function saveState() {
            try {
                window.sessionStorage.setItem(SESSION_KEY, JSON.stringify({
                    open: !panel.hidden,
                    // Remember the window the server asked for, so the first
                    // request after a navigation already uses it.
                    contextExchanges: contextExchanges,
                    exchanges: exchanges.slice(-MAX_EXCHANGES)
                }));
            } catch (err) {
                // sessionStorage may be unavailable; the widget still works.
            }
        }

        function openPanel(focus) {
            if (!panel.hidden) {
                return;
            }
            panel.hidden = false;
            trigger.setAttribute("aria-expanded", "true");
            document.addEventListener("keydown", onKeyDown);
            document.addEventListener("click", onDocumentClick, true);
            saveState();
            if (focus !== false) {
                window.setTimeout(function () {
                    input.focus();
                }, 0);
            }
        }

        function closePanel() {
            if (panel.hidden) {
                return;
            }
            panel.hidden = true;
            trigger.setAttribute("aria-expanded", "false");
            document.removeEventListener("keydown", onKeyDown);
            document.removeEventListener("click", onDocumentClick, true);
            if (controller) {
                controller.abort();
            }
            saveState();
            trigger.focus();
        }

        function onKeyDown(event) {
            if (event.key === "Escape") {
                event.preventDefault();
                closePanel();
                return;
            }
            if (event.key === "Tab") {
                trapFocus(event);
            }
        }

        function onDocumentClick(event) {
            if (!root.contains(event.target)) {
                closePanel();
            }
        }

        function trapFocus(event) {
            var focusable = Array.prototype.slice
                .call(panel.querySelectorAll("button, textarea, a[href]"))
                .filter(function (el) {
                    return !el.disabled && el.offsetParent !== null;
                });
            if (!focusable.length) {
                return;
            }
            var first = focusable[0];
            var last = focusable[focusable.length - 1];
            if (event.shiftKey && document.activeElement === first) {
                event.preventDefault();
                last.focus();
            } else if (!event.shiftKey && document.activeElement === last) {
                event.preventDefault();
                first.focus();
            }
        }

        function contextSize() {
            var completed = 0;
            exchanges.forEach(function (entry) {
                if (entry.kind !== "pending") {
                    completed++;
                }
            });
            return Math.min(completed, contextExchanges);
        }

        function updateContextLabel() {
            var size = contextSize();
            if (size === 0) {
                contextEl.hidden = true;
                contextEl.textContent = "";
                return;
            }
            contextEl.textContent = "Context: last " + size +
                (size === 1 ? " exchange" : " exchanges");
            contextEl.hidden = false;
        }

        // renderTranscript rebuilds the whole conversation. It is only called
        // outside of streaming; during streaming the live exchange is updated
        // in place.
        function renderTranscript() {
            transcript.innerHTML = "";
            live = null;
            empty.hidden = exchanges.length > 0;

            var completed = 0;
            exchanges.forEach(function (entry) {
                if (entry.kind !== "pending") {
                    completed++;
                }
            });
            // Everything before this index is no longer sent to the API.
            var outOfContext = Math.max(0, completed - contextExchanges);

            exchanges.forEach(function (entry, index) {
                if (outOfContext > 0 && index === outOfContext) {
                    var divider = document.createElement("div");
                    divider.className = "ask-ai-boundary";
                    divider.textContent = "Earlier messages are no longer included in the assistant's context";
                    transcript.appendChild(divider);
                }

                var turn = document.createElement("div");
                turn.className = "ask-ai-turn";
                if (index < outOfContext) {
                    turn.className += " ask-ai-turn-stale";
                }

                var question = document.createElement("div");
                question.className = "ask-ai-question";
                question.textContent = entry.question;
                turn.appendChild(question);

                var status = document.createElement("div");
                status.className = "ask-ai-status";
                var spinner = document.createElement("span");
                spinner.className = "ask-ai-spinner";
                spinner.setAttribute("aria-hidden", "true");
                var statusText = document.createElement("span");
                status.appendChild(spinner);
                status.appendChild(statusText);
                status.hidden = true;
                turn.appendChild(status);

                var answer = document.createElement("div");
                answer.className = "ask-ai-answer";
                answer.hidden = true;
                turn.appendChild(answer);

                var sources = document.createElement("div");
                sources.className = "ask-ai-sources";
                sources.hidden = true;
                turn.appendChild(sources);

                transcript.appendChild(turn);

                var item = {
                    entry: entry,
                    status: status,
                    statusText: statusText,
                    answer: answer,
                    sources: sources
                };
                applyEntry(item);

                if (index === exchanges.length - 1) {
                    live = item;
                }
            });

            updateContextLabel();
        }

        function applyEntry(item) {
            var entry = item.entry;
            if (entry.kind === "pending") {
                item.answer.hidden = true;
                item.sources.hidden = true;
                item.statusText.textContent = entry.stage === "answer"
                    ? "Generating answer\u2026"
                    : "Checking relevance\u2026";
                item.status.hidden = false;
                return;
            }

            item.status.hidden = true;
            if (entry.kind === "answer" && entry.answer) {
                item.answer.className = "ask-ai-answer";
                item.answer.innerHTML = renderMarkdown(entry.answer);
                item.answer.hidden = false;
            } else if (entry.kind === "error") {
                item.answer.className = "ask-ai-answer ask-ai-error";
                item.answer.textContent = entry.message || "Something went wrong.";
                item.answer.hidden = false;
            } else {
                item.answer.className = "ask-ai-answer ask-ai-note";
                item.answer.textContent = entry.message || "No answer was returned.";
                item.answer.hidden = false;
            }
            renderSources(item.sources, entry.sources);
        }

        function setStatus(text) {
            if (!live) {
                return;
            }
            live.statusText.textContent = text;
            live.status.hidden = false;
        }

        function setBusy(busy) {
            streaming = busy;
            sendButton.disabled = busy;
            input.disabled = busy;
            stopButton.hidden = !busy;
        }

        function renderNow() {
            if (renderTimer) {
                window.clearTimeout(renderTimer);
                renderTimer = null;
            }
            if (!live) {
                return;
            }
            live.entry.answer = currentAnswer;
            live.answer.className = "ask-ai-answer";
            live.answer.innerHTML = renderMarkdown(currentAnswer);
            live.answer.hidden = false;
            scrollBottom();
        }

        function scheduleRender() {
            if (renderTimer) {
                return;
            }
            renderTimer = window.setTimeout(function () {
                renderTimer = null;
                renderNow();
            }, 80);
        }

        function renderSources(el, urls) {
            el.innerHTML = "";
            if (!urls || !urls.length) {
                el.hidden = true;
                return;
            }
            var title = document.createElement("div");
            title.className = "ask-ai-sources-title";
            title.textContent = "Sources";
            el.appendChild(title);

            var list = document.createElement("ul");
            urls.forEach(function (url) {
                var item = document.createElement("li");
                var link = document.createElement("a");
                link.href = url;
                link.target = "_blank";
                link.rel = "noopener noreferrer";
                link.textContent = prettyUrl(url);
                item.appendChild(link);
                list.appendChild(item);
            });
            el.appendChild(list);
            el.hidden = false;
        }

        function prettyUrl(url) {
            try {
                var parsed = new URL(url);
                var path = parsed.pathname
                    .replace(/^\/+|\/+$/g, "")
                    .replace(/\.md$/, "");
                return path || parsed.hostname;
            } catch (err) {
                return url;
            }
        }

        function scrollBottom() {
            var distance = body.scrollHeight - body.scrollTop - body.clientHeight;
            if (distance < 80) {
                body.scrollTop = body.scrollHeight;
            }
        }

        // restoreState brings back the transcript and the open/closed state
        // from this tab's previous page.
        function restoreState() {
            var state = loadState();
            if (!state) {
                return;
            }

            exchanges = state.exchanges.slice(-MAX_EXCHANGES);
            if (state.contextExchanges !== null) {
                contextExchanges = state.contextExchanges;
            }
            exchanges.forEach(function (entry) {
                if (entry.kind === "pending") {
                    entry.kind = "note";
                    entry.message = "The previous question was interrupted.";
                    entry.stage = "";
                }
            });
            renderTranscript();

            if (state.open) {
                openPanel(false);
            }
        }

        // buildHistory returns the completed exchanges to send as conversation
        // context: the last contextExchanges question/answer pairs, oldest
        // first. Exchanges that never produced an answer are skipped.
        function buildHistory() {
            if (contextExchanges <= 0) {
                return [];
            }

            var completed = exchanges.filter(function (entry) {
                return entry.kind === "answer" || entry.kind === "note";
            });
            completed = completed.slice(-contextExchanges);

            var history = [];
            completed.forEach(function (entry) {
                var content = entry.kind === "answer" ? entry.answer : entry.message;
                if (!content) {
                    return;
                }
                history.push({ role: "user", content: entry.question });
                history.push({ role: "assistant", content: content });
            });
            return history;
        }

        async function ask(question) {
            question = String(question || "").trim();
            if (!question || streaming) {
                return;
            }

            var history = buildHistory();
            exchanges.push({
                question: question,
                kind: "pending",
                answer: "",
                sources: [],
                message: "",
                stage: "relevance"
            });
            if (exchanges.length > MAX_EXCHANGES) {
                exchanges = exchanges.slice(-MAX_EXCHANGES);
            }
            renderTranscript();
            scrollBottom();

            controller = new AbortController();
            setBusy(true);
            currentAnswer = "";
            sawDone = false;
            sawError = false;

            try {
                var payload = { question: question };
                if (history.length) {
                    payload.history = history;
                }

                var response = await fetch(ENDPOINT, {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json",
                        "Accept": "text/event-stream"
                    },
                    body: JSON.stringify(payload),
                    signal: controller.signal
                });

                if (response.status === 429) {
                    throw new Error(rateLimitMessage(response));
                }
                if (!response.ok) {
                    throw new Error(await responseError(response));
                }
                if (!response.body) {
                    throw new Error("This browser does not support streaming responses.");
                }

                await consume(response.body);

                if (!sawDone && !sawError) {
                    if (currentAnswer) {
                        renderNow();
                        finishAnswer([]);
                        setStatus("Connection closed.");
                    } else {
                        finishError("The connection closed before the answer finished.");
                    }
                }
            } catch (err) {
                if (err && err.name === "AbortError") {
                    if (currentAnswer) {
                        renderNow();
                        finishAnswer([]);
                        setStatus("Stopped.");
                    } else {
                        finishNote("Stopped.");
                    }
                } else {
                    finishError((err && err.message) || "Something went wrong. Please try again.");
                }
            } finally {
                controller = null;
                setBusy(false);
            }
        }

        function finishAnswer(sources) {
            if (!live) {
                return;
            }
            live.entry.kind = "answer";
            live.entry.answer = currentAnswer;
            live.entry.sources = Array.isArray(sources) ? sources : [];
            live.entry.message = "";
            live.entry.stage = "";
            // Rebuild so the context boundary and the stale styling follow the
            // exchange that just completed.
            renderTranscript();
            scrollBottom();
            saveState();
        }

        function finishNote(message) {
            if (!live) {
                return;
            }
            live.entry.kind = "note";
            live.entry.answer = "";
            live.entry.sources = [];
            live.entry.message = message;
            live.entry.stage = "";
            currentAnswer = "";
            renderTranscript();
            scrollBottom();
            saveState();
        }

        function finishError(message) {
            if (!live) {
                return;
            }
            live.entry.kind = "error";
            live.entry.answer = "";
            live.entry.sources = [];
            live.entry.message = message;
            live.entry.stage = "";
            currentAnswer = "";
            renderTranscript();
            scrollBottom();
            saveState();
        }

        async function consume(stream) {
            var reader = stream.getReader();
            var decoder = new TextDecoder();
            var buffer = "";

            while (true) {
                var chunk = await reader.read();
                if (chunk.done) {
                    break;
                }
                buffer += decoder.decode(chunk.value, { stream: true });
                buffer = buffer.replace(/\r\n/g, "\n");

                var boundary;
                while ((boundary = buffer.indexOf("\n\n")) !== -1) {
                    var raw = buffer.slice(0, boundary);
                    buffer = buffer.slice(boundary + 2);
                    handleEvent(raw);
                }
            }

            buffer += decoder.decode();
            if (buffer.trim()) {
                handleEvent(buffer);
            }
        }

        function handleEvent(raw) {
            var eventName = "message";
            var dataLines = [];
            raw.split("\n").forEach(function (line) {
                if (line.indexOf("event:") === 0) {
                    eventName = line.slice(6).trim();
                } else if (line.indexOf("data:") === 0) {
                    dataLines.push(line.slice(5).replace(/^ /, ""));
                }
            });
            if (!dataLines.length) {
                return;
            }

            var payload;
            try {
                payload = JSON.parse(dataLines.join("\n"));
            } catch (err) {
                return;
            }

            if (eventName === "relevance") {
                if (typeof payload.history_exchanges === "number" && payload.history_exchanges >= 0) {
                    contextExchanges = payload.history_exchanges;
                }
                if (live) {
                    live.entry.stage = payload.relevant ? "answer" : "relevance";
                }
                if (payload.relevant) {
                    setStatus("Generating answer\u2026");
                } else {
                    setStatus("Not covered by the docs.");
                }
                return;
            }

            if (eventName === "delta") {
                if (payload.text) {
                    currentAnswer += payload.text;
                    scheduleRender();
                }
                return;
            }

            if (eventName === "done") {
                sawDone = true;
                var answer = payload.answer || currentAnswer;
                if (answer) {
                    currentAnswer = answer;
                    finishAnswer(payload.sources);
                } else if (payload.message) {
                    finishNote(payload.message);
                } else {
                    finishNote("No answer was returned.");
                }
                return;
            }

            if (eventName === "error") {
                sawError = true;
                finishError(payload.error || "The answer request failed.");
            }
        }

        async function responseError(response) {
            try {
                var data = await response.json();
                if (data && data.error) {
                    return data.error;
                }
            } catch (err) {
                // fall through
            }
            if (response.status === 502) {
                return "The assistant is temporarily unavailable. Please try again.";
            }
            return "Request failed (" + response.status + ").";
        }

        function rateLimitMessage(response) {
            var retry = response.headers.get("Retry-After");
            if (retry) {
                return "Too many requests. Please try again in " + retry + "s.";
            }
            return "Too many requests. Please try again shortly.";
        }

        function startNewConversation() {
            if (controller) {
                controller.abort();
            }
            exchanges = [];
            currentAnswer = "";
            renderTranscript();
            input.value = "";
            saveState();
            input.focus();
        }

        trigger.addEventListener("click", function () {
            if (panel.hidden) {
                openPanel();
            } else {
                closePanel();
            }
        });
        closeButton.addEventListener("click", closePanel);
        newButton.addEventListener("click", startNewConversation);
        stopButton.addEventListener("click", function () {
            if (controller) {
                controller.abort();
            }
        });

        form.addEventListener("submit", function (event) {
            event.preventDefault();
            var question = input.value;
            if (!question.trim()) {
                return;
            }
            input.value = "";
            ask(question);
        });

        input.addEventListener("keydown", function (event) {
            if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                if (form.requestSubmit) {
                    form.requestSubmit();
                } else {
                    form.dispatchEvent(new Event("submit", { cancelable: true }));
                }
            }
        });

        root.addEventListener("click", function (event) {
            var example = event.target.closest("[data-ask-question]");
            if (!example) {
                return;
            }
            event.preventDefault();
            input.value = example.getAttribute("data-ask-question") || "";
            if (form.requestSubmit) {
                form.requestSubmit();
            } else {
                form.dispatchEvent(new Event("submit", { cancelable: true }));
            }
        });

        restoreState();
    }

    function escapeHtml(value) {
        return String(value).replace(/[&<>"']/g, function (char) {
            return {
                "&": "&amp;",
                "<": "&lt;",
                ">": "&gt;",
                '"': "&quot;",
                "'": "&#39;"
            }[char];
        });
    }

    // Small, dependency-free Markdown renderer for the answer. The source is
    // escaped first, then only a safe subset (headings, lists, links, emphasis,
    // inline code and fenced code) is rendered.
    function renderMarkdown(source) {
        var escaped = escapeHtml(source || "");
        var lines = escaped.split("\n");
        var html = "";
        var textLines = [];

        function flushText() {
            if (!textLines.length) {
                return;
            }
            html += renderBlocks(textLines.join("\n"));
            textLines = [];
        }

        for (var i = 0; i < lines.length; i++) {
            var fence = lines[i].match(/^\s*```\s*([^\s`]*)\s*$/);
            if (!fence) {
                textLines.push(lines[i]);
                continue;
            }

            flushText();
            var language = (fence[1] || "").replace(/[^\w+-]/g, "");
            var code = [];
            i++;
            while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
                code.push(lines[i]);
                i++;
            }
            html += '<pre class="ask-ai-code"><code' +
                (language ? ' class="language-' + language + '"' : "") +
                ">" + code.join("\n") + "</code></pre>";
        }
        flushText();
        return html;
    }

    function renderBlocks(text) {
        var lines = text.split("\n");
        var html = "";
        var paragraph = [];
        var list = null;

        function flushParagraph() {
            if (!paragraph.length) {
                return;
            }
            html += "<p>" + renderInline(paragraph.join(" ")) + "</p>";
            paragraph = [];
        }

        function flushList() {
            if (!list) {
                return;
            }
            html += "<ul>" + list.map(function (item) {
                return "<li>" + renderInline(item) + "</li>";
            }).join("") + "</ul>";
            list = null;
        }

        lines.forEach(function (rawLine) {
            var line = rawLine.trim();
            if (!line) {
                flushParagraph();
                flushList();
                return;
            }

            var heading = line.match(/^(#{1,4})\s+(.*)$/);
            if (heading) {
                flushParagraph();
                flushList();
                var level = Math.min(heading[1].length + 2, 6);
                html += "<h" + level + ">" + renderInline(heading[2]) + "</h" + level + ">";
                return;
            }

            var bullet = line.match(/^[-*]\s+(.*)$/);
            if (bullet) {
                flushParagraph();
                if (!list) {
                    list = [];
                }
                list.push(bullet[1]);
                return;
            }

            var numbered = line.match(/^\d+\.\s+(.*)$/);
            if (numbered) {
                flushParagraph();
                if (!list) {
                    list = [];
                }
                list.push(numbered[1]);
                return;
            }

            flushList();
            paragraph.push(line);
        });

        flushParagraph();
        flushList();
        return html;
    }

    function renderInline(text) {
        var codes = [];
        text = text.replace(/`([^`]+)`/g, function (_, code) {
            codes.push(code);
            return "\u0000" + (codes.length - 1) + "\u0000";
        });
        text = text.replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, function (_, label, url) {
            return '<a href="' + url + '" target="_blank" rel="noopener noreferrer">' + label + "</a>";
        });
        text = text.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
        text = text.replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
        text = text.replace(/\u0000(\d+)\u0000/g, function (_, index) {
            return "<code>" + codes[Number(index)] + "</code>";
        });
        return text;
    }

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", init);
    } else {
        init();
    }
})();
