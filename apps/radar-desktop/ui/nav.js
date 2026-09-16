// pages in config is visibility and order only. Which columns, cards,
// and sections a page shows stay fixed here. This is not the start of
// a page builder.

// Tauri maps Rust snake_case command arguments to camelCase JS keys.
// page_nav(page_key: String) must be invoked as { pageKey }, not { page_key }.
// Do not send both keys in one call: an unknown extra key fails the invoke.

function invoke(name, args) {
  return window.__TAURI__.core.invoke(name, args).then(
    (result) => {
      console.log(name, "ok", result);
      return result;
    },
    (err) => {
      console.error(name, "error", err);
      throw err;
    }
  );
}

function formatCommandError(err) {
  if (err === null || err === undefined || err === "") {
    return "(empty error)";
  }
  if (typeof err === "string") {
    return err;
  }
  if (typeof err === "number" || typeof err === "boolean") {
    return String(err);
  }
  if (err instanceof Error) {
    return err.message || String(err);
  }
  try {
    const json = JSON.stringify(err);
    if (json && json !== "{}") {
      return json;
    }
  } catch (_e) {
    // fall through
  }
  return String(err);
}

function commandErrorElement() {
  const main = document.querySelector("main");
  let el = document.getElementById("command-error");
  if (el && main && el.parentElement !== main) {
    main.insertBefore(el, main.firstChild);
  } else if (!el && main) {
    el = document.createElement("p");
    el.id = "command-error";
    main.insertBefore(el, main.firstChild);
  }
  if (el) {
    el.className = "error";
  }
  return el;
}

function showCommandFailure(commandName, err) {
  const el = commandErrorElement();
  const text = commandName + ": " + formatCommandError(err);
  console.error(text, err);
  if (!el) {
    return;
  }
  el.removeAttribute("hidden");
  el.hidden = false;
  el.style.display = "block";
  el.className = "error";
  el.setAttribute("role", "alert");
  if (commandName === "page_nav") {
    el.dataset.sticky = "page_nav";
  }
  el.textContent = text;
}

function clearCommandFailure() {
  const el = document.getElementById("command-error");
  if (!el) {
    return;
  }
  if (el.dataset.sticky === "page_nav") {
    return;
  }
  el.hidden = true;
  el.style.display = "";
  el.textContent = "";
}

function pickField(obj, snake, camel) {
  if (!obj || typeof obj !== "object") {
    return undefined;
  }
  if (obj[snake] !== undefined) {
    return obj[snake];
  }
  return obj[camel];
}

window.addEventListener("error", (event) => {
  showCommandFailure("script", event.message || String(event.error || "error"));
});
window.addEventListener("unhandledrejection", (event) => {
  showCommandFailure("unhandledrejection", event.reason);
});

async function invokePageNav(pageKey) {
  try {
    return await invoke("page_nav", { pageKey: pageKey });
  } catch (err) {
    try {
      return await invoke("page_nav", { page_key: pageKey });
    } catch (_err2) {
      throw err;
    }
  }
}

async function applyConfiguredNav() {
  const pageKey = document.body.dataset.pageKey;
  const navEl = document.querySelector(".sidebar-nav");
  const disabled = document.getElementById("page-disabled");
  const body = document.getElementById("page-body");
  if (!pageKey) {
    showCommandFailure(
      "page_nav",
      "body data-page-key is missing; nav cannot be requested"
    );
    return true;
  }
  let nav;
  try {
    nav = await invokePageNav(pageKey);
  } catch (err) {
    showCommandFailure("page_nav", err);
    return true;
  }
  const pages = Array.isArray(pickField(nav, "pages", "pages"))
    ? pickField(nav, "pages", "pages")
    : [];
  if (!Array.isArray(nav && nav.pages) && !Array.isArray(nav && nav.Pages)) {
    showCommandFailure(
      "page_nav",
      "response had no pages array: " + JSON.stringify(nav)
    );
  }
  console.log("nav render input pageKey=", pageKey, "pages=", pages);
  if (navEl) {
    navEl.innerHTML = "";
    for (const page of pages) {
      const a = document.createElement("a");
      a.className = "nav-item";
      const key = pickField(page, "key", "key");
      const href = pickField(page, "href", "href");
      const label = pickField(page, "label", "label");
      if (key === pageKey) {
        a.classList.add("nav-item-active");
      }
      a.href = href || "#";
      a.textContent = label || key || "";
      navEl.appendChild(a);
    }
  }
  const enabled = pickField(nav, "enabled", "enabled");
  if (enabled === false) {
    if (disabled) {
      disabled.hidden = false;
      disabled.textContent =
        pickField(nav, "disabled_text", "disabledText") ||
        "This page is not enabled for this configuration.";
    }
    if (body) body.hidden = true;
    return false;
  }
  return true;
}
