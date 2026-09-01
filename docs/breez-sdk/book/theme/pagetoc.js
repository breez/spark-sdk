/**
 * Page Table of Contents (TOC) Manager
 * Handles the dynamic sidebar navigation for the current page
 */
(function () {
  "use strict";

  let scrollTimeout = null;
  const SCROLL_COOLDOWN = 150; // ms to wait after manual click before auto-updating
  const SCROLL_OFFSET = 60; // px offset for active section detection

  /**
   * Attach click listeners to TOC links
   */
  const attachClickListeners = () => {
    const pagetoc = document.querySelector(".pagetoc");
    if (!pagetoc) return;

    const links = [...pagetoc.children];
    links.forEach((link) => {
      link.addEventListener("click", (event) => {
        // Clear any existing timeout
        if (scrollTimeout) clearTimeout(scrollTimeout);

        // Set active state immediately on click
        links.forEach((l) => l.classList.remove("active"));
        link.classList.add("active");

        // Prevent auto-update during smooth scroll
        scrollTimeout = setTimeout(() => {
          scrollTimeout = null;
        }, SCROLL_COOLDOWN);
      });
    });
  };

  /**
   * Get or create the page TOC container
   */
  const getPageToc = () => {
    return document.querySelector(".pagetoc") || createPageToc();
  };

  /**
   * Create the page TOC container if it doesn't exist
   */
  const createPageToc = () => {
    const main = document.querySelector("#content > main");
    if (!main) return null;

    // Wrap existing content
    const contentWrap = document.createElement("div");
    contentWrap.className = "content-wrap";
    contentWrap.append(...main.childNodes);
    main.appendChild(contentWrap);

    // Insert TOC container at the beginning
    main.insertAdjacentHTML(
      "afterbegin",
      '<div class="sidetoc"><nav class="pagetoc" aria-label="Page navigation"></nav></div>'
    );

    return document.querySelector(".pagetoc");
  };

  /**
   * Update active TOC link based on scroll position
   */
  const updateActiveTocLink = () => {
    // Skip if we're in cooldown after a manual click
    if (scrollTimeout) return;

    const pagetoc = document.querySelector(".pagetoc");
    if (!pagetoc) return;

    const headers = [...document.getElementsByClassName("header")];
    const scrollPosition = window.scrollY + SCROLL_OFFSET;
    let activeHeader = null;

    // Find the last header that's above the current scroll position
    for (let i = headers.length - 1; i >= 0; i--) {
      const headerTop = headers[i].offsetTop;
      if (scrollPosition >= headerTop) {
        activeHeader = headers[i];
        break;
      }
    }

    // Update active states
    const tocLinks = [...pagetoc.children];
    tocLinks.forEach((link) => link.classList.remove("active"));

    if (activeHeader) {
      const activeLink = tocLinks.find(
        (link) => link.href === activeHeader.href
      );
      if (activeLink) {
        activeLink.classList.add("active");

        // Ensure the active link is visible in the TOC
        activeLink.scrollIntoView({
          block: "nearest",
          behavior: "smooth",
        });
      }
    }
  };

  /**
   * Populate the TOC with links to page headers
   */
  const populateToc = () => {
    const pagetoc = getPageToc();
    if (!pagetoc) return;

    const headers = [...document.getElementsByClassName("header")];

    headers.forEach((header) => {
      const parent = header.parentElement;

      // Skip headers marked with toc-ignore class
      if (!parent || parent.classList.contains("toc-ignore")) {
        return;
      }

      // Skip headers inside warning/note boxes or other special containers
      if (parent.closest(".warning, .note, blockquote")) {
        return;
      }

      // Extract text content from the header (excluding tags and other non-header elements)
      const textContent = [...parent.childNodes]
        .filter((node) => {
          // Exclude elements with 'tag' class (like API docs tags)
          if (
            node.nodeType === Node.ELEMENT_NODE &&
            node.classList.contains("tag")
          ) {
            return false;
          }
          return true;
        })
        .map((node) => node.textContent || "")
        .join("")
        .trim();

      if (!textContent) return;

      // Create TOC link
      const link = document.createElement("a");
      link.textContent = textContent;
      link.href = header.href;
      link.className = `pagetoc-${parent.tagName}`;

      pagetoc.appendChild(link);
    });
  };

  /**
   * Initialize the page TOC
   */
  const initPageToc = () => {
    populateToc();
    attachClickListeners();
    updateActiveTocLink();

    // Listen for scroll events
    let rafId = null;
    window.addEventListener(
      "scroll",
      () => {
        // Use requestAnimationFrame for better performance
        if (rafId) return;
        rafId = requestAnimationFrame(() => {
          updateActiveTocLink();
          rafId = null;
        });
      },
      { passive: true }
    );
  };

  // Initialize when DOM is ready
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initPageToc);
  } else {
    initPageToc();
  }
})();
