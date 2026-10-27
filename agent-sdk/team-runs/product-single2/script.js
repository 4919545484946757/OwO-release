/**
 * OwO Agent SDK — landing page behaviour.
 *
 * Kept deliberately tiny and side-effect free on import so it can be unit
 * tested with node:test (see test/landing.test.mjs). All DOM access happens
 * inside initNav(), which only runs in the browser.
 */

/**
 * Pure state machine for the mobile navigation toggle.
 * @param {boolean} isExpanded current aria-expanded value
 * @param {boolean} [force] optional explicit target state
 * @returns {boolean} the next aria-expanded value
 */
export function nextExpandedState(isExpanded, force) {
  if (typeof force === "boolean") return force;
  return !isExpanded;
}

/**
 * Wire up the mobile navigation toggle in a progressive-enhancement friendly
 * way: the markup is fully usable without JavaScript (CSS shows the nav at
 * desktop widths), and JS only adds the collapsed/expanded behaviour.
 * @param {Document} doc
 * @returns {{destroy: () => void, toggle: (force?: boolean) => boolean} | null}
 */
export function initNav(doc) {
  const button = doc.querySelector("[data-nav-toggle]");
  const nav = doc.querySelector("[data-nav]");
  if (!button || !nav) return null;

  const setExpanded = (expanded) => {
    button.setAttribute("aria-expanded", String(expanded));
    nav.classList.toggle("is-open", expanded);
    return expanded;
  };

  const toggle = (force) => {
    const current = button.getAttribute("aria-expanded") === "true";
    return setExpanded(nextExpandedState(current, force));
  };

  const onClick = () => toggle();

  const onKeydown = (event) => {
    if (event.key === "Escape" && button.getAttribute("aria-expanded") === "true") {
      setExpanded(false);
      button.focus();
    }
  };

  const onNavigate = (event) => {
    const target = event.target;
    if (target instanceof doc.defaultView.HTMLElement && target.closest("a")) {
      setExpanded(false);
    }
  };

  button.addEventListener("click", onClick);
  doc.addEventListener("keydown", onKeydown);
  nav.addEventListener("click", onNavigate);

  // Start from the state declared in the markup.
  const initiallyExpanded = button.getAttribute("aria-expanded") === "true";
  setExpanded(initiallyExpanded);

  return {
    toggle,
    destroy() {
      button.removeEventListener("click", onClick);
      doc.removeEventListener("keydown", onKeydown);
      nav.removeEventListener("click", onNavigate);
    },
  };
}

/**
 * Collapse open FAQ items when another one opens, so the page never grows into
 * an endlessly long accordion on small screens.
 * @param {Document} doc
 */
export function initFaqAccordion(doc) {
  const items = Array.from(doc.querySelectorAll(".faq-item"));
  if (items.length === 0) return;

  items.forEach((item) => {
    item.addEventListener("toggle", () => {
      if (!item.open) return;
      items.forEach((other) => {
        if (other !== item) other.open = false;
      });
    });
  });
}

if (typeof document !== "undefined" && typeof window !== "undefined") {
  const start = () => {
    initNav(document);
    initFaqAccordion(document);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
}
