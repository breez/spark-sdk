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
// Markdown answer. Each question is answered independently; there is no
// conversation context.
(function () {
    "use strict";

    var ENDPOINT = "/ask-api/ask/stream";
    var SESSION_KEY = "ask-ai-state-v1";

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
            '    </div>',
            '    <button class="ask-ai-icon" id="ask-ai-close" type="button" aria-label="Close Ask AI">&times;</button>',
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
            '    <div class="ask-ai-turn" id="ask-ai-turn" hidden>',
            '      <div class="ask-ai-question" id="ask-ai-question"></div>',
            '      <div class="ask-ai-status" id="ask-ai-status" hidden>',
            '        <span class="ask-ai-spinner" aria-hidden="true"></span>',
            '        <span id="ask-ai-status-text"></span>',
            '      </div>',
            '      <div class="ask-ai-answer" id="ask-ai-answer" hidden></div>',
            '      <div class="ask-ai-sources" id="ask-ai-sources" hidden></div>',
            '    </div>',
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
        var body = document.getElementById("ask-ai-body");
        var empty = document.getElementById("ask-ai-empty");
        var turn = document.getElementById("ask-ai-turn");
        var questionEl = document.getElementById("ask-ai-question");
        var statusEl = document.getElementById("ask-ai-status");
        var statusText = document.getElementById("ask-ai-status-text");
        var answerEl = document.getElementById("ask-ai-answer");
        var sourcesEl = document.getElementById("ask-ai-sources");
        var form = document.getElementById("ask-ai-form");
        var input = document.getElementById("ask-ai-input");
        var sendButton = document.getElementById("ask-ai-send");
        var stopButton = document.getElementById("ask-ai-stop");

        var controller = null;
        var streaming = false;
        var currentAnswer = "";
        var renderTimer = null;
        var sawDone = false;
        var sawError = false;
        var lastSources = [];
        var lastKind = "";
        var lastMessage = "";

        function loadSession() {
            try {
                var raw = window.sessionStorage.getItem(SESSION_KEY);
                if (!raw) {
                    return null;
                }
                var state = JSON.parse(raw);
                return state && typeof state === "object" ? state : null;
            } catch (err) {
                return null;
            }
        }

        function saveCurrent() {
            var state = {
                open: !panel.hidden,
                question: questionEl.textContent || "",
                answer: currentAnswer || "",
                sources: lastSources || [],
                kind: lastKind || "",
                message: lastMessage || ""
            };
            try {
                window.sessionStorage.setItem(SESSION_KEY, JSON.stringify(state));
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
            saveCurrent();
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
            saveCurrent();
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

        function setStatus(text) {
            statusText.textContent = text;
            statusEl.hidden = false;
        }

        function setBusy(busy) {
            streaming = busy;
            sendButton.disabled = busy;
            input.disabled = busy;
            stopButton.hidden = !busy;
        }

        function resetTurn(question) {
            empty.hidden = true;
            turn.hidden = false;
            questionEl.textContent = question;
            answerEl.className = "ask-ai-answer";
            answerEl.innerHTML = "";
            answerEl.hidden = true;
            sourcesEl.innerHTML = "";
            sourcesEl.hidden = true;
            currentAnswer = "";
            lastSources = [];
            lastKind = "pending";
            lastMessage = "";
            sawDone = false;
            body.scrollTop = 0;
            sawError = false;
            setStatus("Checking relevance\u2026");
            saveCurrent();
        }

        function showError(message) {
            lastKind = "error";
            lastMessage = message;
            lastSources = [];
            answerEl.className = "ask-ai-answer ask-ai-error";
            answerEl.textContent = message;
            answerEl.hidden = false;
            statusEl.hidden = true;
            scrollBottom();
            saveCurrent();
        }

        function showNote(message) {
            lastKind = "note";
            lastMessage = message;
            lastSources = [];
            answerEl.className = "ask-ai-answer ask-ai-note";
            answerEl.textContent = message;
            answerEl.hidden = false;
            statusEl.hidden = true;
            scrollBottom();
            saveCurrent();
        }

        function renderNow() {
            if (renderTimer) {
                window.clearTimeout(renderTimer);
                renderTimer = null;
            }
            answerEl.className = "ask-ai-answer";
            answerEl.innerHTML = renderMarkdown(currentAnswer);
            answerEl.hidden = false;
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

        function renderSources(urls) {
            lastSources = Array.isArray(urls) ? urls.slice() : [];
            sourcesEl.innerHTML = "";
            if (!lastSources.length) {
                sourcesEl.hidden = true;
                return;
            }
            var title = document.createElement("div");
            title.className = "ask-ai-sources-title";
            title.textContent = "Sources";
            sourcesEl.appendChild(title);

            var list = document.createElement("ul");
            lastSources.forEach(function (url) {
                var item = document.createElement("li");
                var link = document.createElement("a");
                link.href = url;
                link.target = "_blank";
                link.rel = "noopener noreferrer";
                link.textContent = prettyUrl(url);
                item.appendChild(link);
                list.appendChild(item);
            });
            sourcesEl.appendChild(list);
            sourcesEl.hidden = false;
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

        function restoreSession() {
            var state = loadSession();
            if (!state) {
                return;
            }

            var question = typeof state.question === "string" ? state.question : "";
            if (question) {
                empty.hidden = true;
                turn.hidden = false;
                questionEl.textContent = question;
                currentAnswer = typeof state.answer === "string" ? state.answer : "";
                lastSources = Array.isArray(state.sources) ? state.sources.slice() : [];
                lastKind = typeof state.kind === "string" ? state.kind : "";
                lastMessage = typeof state.message === "string" ? state.message : "";

                if (lastKind === "answer" && currentAnswer) {
                    renderNow();
                } else if (lastKind === "error") {
                    showError(lastMessage || "Something went wrong.");
                } else if (lastKind === "pending") {
                    showNote("The previous question was interrupted.");
                } else if (lastMessage) {
                    showNote(lastMessage);
                } else {
                    showNote("No answer was returned.");
                }
                renderSources(lastSources);
                statusEl.hidden = true;
            }

            if (state.open) {
                openPanel(false);
            }
        }

        async function ask(question) {
            question = String(question || "").trim();
            if (!question || streaming) {
                return;
            }

            controller = new AbortController();
            setBusy(true);
            resetTurn(question);

            try {
                var response = await fetch(ENDPOINT, {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json",
                        "Accept": "text/event-stream"
                    },
                    body: JSON.stringify({ question: question }),
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
                        lastKind = "answer";
                        lastMessage = "";
                        saveCurrent();
                        setStatus("Connection closed.");
                    } else {
                        showError("The connection closed before the answer finished.");
                    }
                }
            } catch (err) {
                if (err && err.name === "AbortError") {
                    if (currentAnswer) {
                        renderNow();
                        lastKind = "answer";
                        lastMessage = "";
                        saveCurrent();
                        setStatus("Stopped.");
                    } else {
                        showNote("Stopped.");
                    }
                } else {
                    showError((err && err.message) || "Something went wrong. Please try again.");
                }
            } finally {
                controller = null;
                setBusy(false);
            }
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
                if (payload.answer) {
                    currentAnswer = payload.answer;
                }
                if (currentAnswer) {
                    lastKind = "answer";
                    lastMessage = "";
                    renderNow();
                } else if (payload.message) {
                    lastKind = "note";
                    lastMessage = payload.message;
                    showNote(payload.message);
                } else {
                    lastKind = "note";
                    lastMessage = "No answer was returned.";
                    showNote("No answer was returned.");
                }
                renderSources(payload.sources || []);
                statusEl.hidden = true;
                saveCurrent();
                return;
            }

            if (eventName === "error") {
                sawError = true;
                showError(payload.error || "The answer request failed.");
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

        trigger.addEventListener("click", function () {
            if (panel.hidden) {
                openPanel();
            } else {
                closePanel();
            }
        });
        closeButton.addEventListener("click", closePanel);
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

        restoreSession();
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
