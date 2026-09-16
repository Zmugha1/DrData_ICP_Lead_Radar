/*
 * A tooltip is optional reading. A caveat is required rendering.
 *
 * zones.toml carries display_caveat on the Below Zone assignment for
 * prospect_ranking_by_likelihood_to_buy. That text is always on
 * screen wherever a ranking appears. It never becomes a tooltip and
 * it is never behind a click.
 *
 * TermInfo is glossary help only. Do not use it for display_caveat.
 * The owner groups page renders display_caveat above the list.
 */

class TermInfo extends HTMLElement {
  constructor() {
    super();
    this._open = false;
    this._onDocKey = (event) => {
      if (event.key === "Escape") {
        this.close();
      }
    };
  }

  connectedCallback() {
    this.classList.add("term-info");
    const key = this.getAttribute("term-key") || "";
    this.innerHTML = "";

    this.marker = document.createElement("button");
    this.marker.type = "button";
    this.marker.className = "term-info-marker";
    this.marker.setAttribute("aria-expanded", "false");
    this.marker.setAttribute("aria-label", "About this term");
    this.marker.textContent = "i";

    this.panel = document.createElement("div");
    this.panel.className = "term-info-panel";
    this.panel.hidden = true;
    this.panel.setAttribute("role", "dialog");

    this.marker.addEventListener("click", () => this.toggle());
    this.marker.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        this.close();
      }
    });

    this.appendChild(this.marker);
    this.appendChild(this.panel);
    this.load(key);
  }

  disconnectedCallback() {
    document.removeEventListener("keydown", this._onDocKey);
  }

  async load(key) {
    if (!key) {
      this.renderMissing("(no term_key)");
      return;
    }
    try {
      const result = await invoke("glossary", { key });
      if (result.kind === "Term") {
        this.renderTerm(result.term);
      } else if (result.kind === "Missing") {
        this.renderMissing(result.term_key);
      } else {
        this.renderMissing(key);
      }
    } catch (err) {
      this.renderMissing(key + " (command failed)");
    }
  }

  renderTerm(term) {
    this.panel.innerHTML = "";
    const label = document.createElement("p");
    label.className = "term-info-label";
    label.textContent = term.label;
    const body = document.createElement("p");
    body.className = "term-info-body";
    body.textContent = term.body;
    this.panel.appendChild(label);
    this.panel.appendChild(body);
    this.marker.setAttribute("aria-label", "About " + term.label);
  }

  renderMissing(termKey) {
    this.panel.innerHTML = "";
    const missing = document.createElement("p");
    missing.className = "term-info-missing";
    missing.textContent = "Term not found: " + termKey;
    this.panel.appendChild(missing);
  }

  toggle() {
    if (this._open) {
      this.close();
    } else {
      this.open();
    }
  }

  open() {
    this._open = true;
    this.panel.hidden = false;
    this.marker.setAttribute("aria-expanded", "true");
    document.addEventListener("keydown", this._onDocKey);
    this.marker.focus();
  }

  close() {
    this._open = false;
    this.panel.hidden = true;
    this.marker.setAttribute("aria-expanded", "false");
    document.removeEventListener("keydown", this._onDocKey);
    this.marker.focus();
  }
}

customElements.define("term-info", TermInfo);
