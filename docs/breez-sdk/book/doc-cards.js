// Renders plain markdown lists as cards on the HTML pages, in the style of
// breez.technology/sdk. The markdown stays an ordinary list, so the .md and
// llms.txt outputs read the same; only the HTML page gets the richer layout.
//
// - The "Key Features" checklist becomes a grid of linked icon cards, the
//   same set as breez.technology/sdk.
// - A list of links preceded by `<!-- cards: tiles -->` becomes a tile grid.
(function () {
    'use strict';

    // Icons from the feature cards on breez.technology/sdk, keyed by card
    // title. Colour comes from styles.css via currentColor.
    const ICONS = {
        "Fully-featured Lightning": '<path d="M18 4L8 18H16L14 28L24 14H16L18 4Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none" opacity="0.8"/>',
        "Languages & Frameworks": '<path d="M10 8L4 16L10 24" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.7"/><path d="M22 8L28 16L22 24" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.7"/><line x1="18" y1="6" x2="14" y2="26" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/>',
        "Passkey Login": '<path d="M16 8C11.6 8 8 11.6 8 16" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.5"/><path d="M16 8C20.4 8 24 11.6 24 16" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.5"/><path d="M16 11C13.2 11 11 13.2 11 16C11 18.8 13.2 21 16 21" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.7"/><path d="M16 11C18.8 11 21 13.2 21 16C21 17.5 20.5 18.8 19.5 19.8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.7"/><circle cx="16" cy="16" r="2" fill="currentColor" opacity="0.9"/>',
        "USDT & USDC": '<circle cx="12" cy="16" r="7" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><circle cx="20" cy="16" r="7" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/>',
        "Stable Balance": '<path d="M8 20L16 16L24 20" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.5"/><path d="M8 16L16 12L24 16" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.7"/><path d="M12 10H20" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/><path d="M10 7L16 4L22 7" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.5"/>',
        "Instant Deposits": '<path d="M6 18V24C6 25.1 6.9 26 8 26H24C25.1 26 26 25.1 26 24V18" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.5"/><line x1="16" y1="5" x2="16" y2="19" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/><path d="M11 14L16 19L21 14" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.8"/>',
        "Multi-device/app Sync": '<rect x="4" y="8" width="10" height="16" rx="2" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.6"/><rect x="18" y="8" width="10" height="16" rx="2" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.6"/><path d="M13 13L15.5 11L13 9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.8"/><path d="M19 19L16.5 21L19 23" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.8"/>',
        "Contacts API": '<circle cx="16" cy="11" r="4" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.7"/><path d="M8 25C8 20.6 11.6 17 16 17C20.4 17 24 20.6 24 25" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.5"/>',
        "Caching & Persistence": '<ellipse cx="16" cy="10" rx="10" ry="3" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><path d="M6 10V16C6 17.7 10.5 19 16 19C21.5 19 26 17.7 26 16V10" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><path d="M6 16V22C6 23.7 10.5 25 16 25C21.5 25 26 23.7 26 22V16" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><path d="M12 17L15 20L21 14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.9"/>',
        "External Signer": '<path d="M16 4L6 9V16C6 22 10.4 27.2 16 28.5C21.6 27.2 26 22 26 16V9L16 4Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none" opacity="0.5"/><circle cx="16" cy="14" r="3" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.8"/><line x1="16" y1="17" x2="16" y2="22" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/><line x1="16" y1="20" x2="18.5" y2="20" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/>',
        "Turnkey Signer": '<circle cx="11" cy="16" r="5" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.8"/><circle cx="11" cy="16" r="1.6" fill="currentColor" opacity="0.9"/><path d="M16 16H27" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/><path d="M23 16V20M27 16V19" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/>',
        "Integrated On-ramps": '<rect x="6" y="8" width="20" height="14" rx="2" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><line x1="6" y1="13" x2="26" y2="13" stroke="currentColor" stroke-width="1.4" opacity="0.4"/><path d="M16 19L19 25" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.7"/><path d="M16 19L13 25" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.7"/><path d="M14 23H18" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.7"/>',
        "Fiat Currencies": '<circle cx="16" cy="16" r="10" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><path d="M19 12.5C18.3 11.6 17.2 11 16 11C14.3 11 13 12.1 13 13.5C13 14.9 14.3 16 16 16C17.7 16 19 17.1 19 18.5C19 19.9 17.7 21 16 21C14.8 21 13.7 20.4 13 19.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none" opacity="0.8"/><line x1="16" y1="9" x2="16" y2="11" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.6"/><line x1="16" y1="21" x2="16" y2="23" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.6"/>',
        "Multi-user Server Mode": '<rect x="9" y="19" width="14" height="8" rx="1.5" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.5"/><line x1="9" y1="23" x2="23" y2="23" stroke="currentColor" stroke-width="1.4" opacity="0.4"/><line x1="12" y1="21" x2="14" y2="21" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/><line x1="12" y1="25" x2="14" y2="25" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.8"/><circle cx="8" cy="8" r="2.4" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.7"/><circle cx="16" cy="6.5" r="2.4" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.7"/><circle cx="24" cy="8" r="2.4" stroke="currentColor" stroke-width="1.4" fill="none" opacity="0.7"/><line x1="8.8" y1="10.2" x2="12" y2="19" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/><line x1="16" y1="9" x2="16" y2="19" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/><line x1="23.2" y1="10.2" x2="20" y2="19" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity="0.5"/>',
    };
    const CHECK = '<path d="M9 16.5L14 21.5L23 11" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>';
    const ARROW = '<svg class="feature-card-arrow" width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M5 11L11 5M11 5H6M11 5V10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';

    // Only the list directly under a "Key Features" label is converted. Each
    // item is `- [x] **[Title](link)**: description`; the whole card links.
    document.querySelectorAll('.content main ul').forEach((ul) => {
        const label = ul.previousElementSibling;
        if (!label || label.textContent.trim() !== 'Key Features') return;
        const items = Array.from(ul.children);
        if (!items.every((li) => li.querySelector(':scope > input[type="checkbox"]'))) return;

        ul.classList.add('feature-cards');
        items.forEach((li) => {
            li.querySelector(':scope > input[type="checkbox"]').remove();
            const link = li.querySelector('strong > a');
            const title = (link || li.querySelector('strong') || li).textContent.trim();
            if (link) link.closest('strong').remove();
            const desc = li.textContent.replace(/^\s*[:\u2013\u2014-]\s*/, '').trim();

            const card = document.createElement(link ? 'a' : 'div');
            card.className = 'feature-card';
            if (link) {
                card.href = link.getAttribute('href');
                if (/^https?:/.test(card.getAttribute('href'))) {
                    card.target = '_blank';
                    card.rel = 'noopener noreferrer';
                }
            }
            card.innerHTML = (link ? ARROW : '') +
                '<svg class="feature-card-icon" width="28" height="28" viewBox="0 0 32 32" fill="none" aria-hidden="true">' + (ICONS[title] || CHECK) + '</svg>' +
                '<span class="feature-card-title"></span><span class="feature-card-desc"></span>';
            card.querySelector('.feature-card-title').textContent = title;
            card.querySelector('.feature-card-desc').textContent = desc;
            li.replaceChildren(card);
        });
    });

    // Lists opted in with a `<!-- cards: kind -->` comment on the line above.
    const markers = [];
    const walker = document.createTreeWalker(document.querySelector('.content main') || document.body, NodeFilter.SHOW_COMMENT);
    while (walker.nextNode()) markers.push(walker.currentNode);
    markers.forEach((marker) => {
        const kind = (marker.textContent.match(/^\s*cards:\s*(tiles)\s*$/) || [])[1];
        const list = marker.nextElementSibling;
        if (!kind || !list || !/^(UL|OL)$/.test(list.tagName)) return;

        if (!Array.from(list.children).every((li) => li.querySelector(':scope > a'))) return;
        list.classList.add('doc-tiles');
        // Let long slash-separated names wrap at the slash on narrow screens.
        list.querySelectorAll(':scope > li > a').forEach((a) => {
            const parts = a.textContent.split('/');
            a.replaceChildren(...parts.flatMap((part, i) => (i < parts.length - 1 ? [part + '/', document.createElement('wbr')] : [part])));
        });
    });
})();
