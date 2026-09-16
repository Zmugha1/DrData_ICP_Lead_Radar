function unknownSpan() {
  const mark = document.createElement("span");
  mark.className = "unknown";
  mark.textContent = "unknown";
  return mark;
}

function appendValue(parent, value) {
  if (value === null || value === undefined) {
    parent.appendChild(unknownSpan());
    return;
  }
  parent.appendChild(document.createTextNode(String(value)));
}

function valueCell(value) {
  const td = document.createElement("td");
  appendValue(td, value);
  return td;
}

function renderCard(card) {
  const section = document.createElement("section");
  section.className = "card card-lg";

  const kicker = document.createElement("p");
  kicker.className = "card-kicker";
  kicker.textContent = card.series_id;
  section.appendChild(kicker);

  const title = document.createElement("h2");
  title.className = "card-title";
  title.textContent = card.label;
  section.appendChild(title);

  const meta = document.createElement("p");
  meta.className = "slate";
  meta.textContent =
    "geography " +
    card.geography +
    ". observed frequency " +
    card.observed_frequency +
    ".";
  section.appendChild(meta);

  if (card.history_text) {
    const history = document.createElement("p");
    history.className = "slate";
    history.textContent = card.history_text;
    section.appendChild(history);
  }

  if (card.stale_text) {
    const stale = document.createElement("p");
    stale.className = "slate";
    stale.textContent = card.stale_text;
    section.appendChild(stale);
  }

  const latest = document.createElement("p");
  latest.className = "card-body";
  if (!card.latest_observation_date) {
    latest.textContent = "No observations in the store.";
  } else {
    latest.appendChild(document.createTextNode("Latest value "));
    appendValue(latest, card.latest_value);
    latest.appendChild(
      document.createTextNode(" on " + card.latest_observation_date + ".")
    );
    if (card.days_old !== null && card.days_old !== undefined) {
      latest.appendChild(
        document.createTextNode(" " + String(card.days_old) + " days old.")
      );
    }
  }
  section.appendChild(latest);

  if (card.recent && card.recent.length > 0) {
    const wrap = document.createElement("div");
    wrap.className = "table-wrap";
    const table = document.createElement("table");
    const thead = document.createElement("thead");
    const headRow = document.createElement("tr");
    const dateHead = document.createElement("th");
    dateHead.textContent = "Observation date";
    const valueHead = document.createElement("th");
    valueHead.textContent = "Value";
    headRow.appendChild(dateHead);
    headRow.appendChild(valueHead);
    thead.appendChild(headRow);
    table.appendChild(thead);
    const tbody = document.createElement("tbody");
    for (const row of card.recent) {
      const tr = document.createElement("tr");
      const dateTd = document.createElement("td");
      dateTd.textContent = row.observation_date;
      tr.appendChild(dateTd);
      tr.appendChild(valueCell(row.value));
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    wrap.appendChild(table);
    section.appendChild(wrap);
  }

  return section;
}

async function loadPage() {
  clearCommandFailure();
  document.getElementById("error").textContent = "";
  document.getElementById("empty-result").textContent = "";
  const root = document.getElementById("cards");
  root.innerHTML = "";
  let page;
  try {
    page = await invoke("market_watch");
  } catch (err) {
    showCommandFailure("market_watch", err);
    return;
  }
  document.getElementById("page-title").textContent =
    "Market watch · " + page.vertical_display_name;
  document.getElementById("config-line").textContent =
    page.county_name +
    " (" +
    page.county_fips +
    "). Config version " +
    page.config_version +
    ".";
  document.getElementById("clock-note").textContent = page.clock_note;
  document.getElementById("grain-note").textContent = page.grain_note;
  if (!page.cards || page.cards.length === 0) {
    document.getElementById("empty-result").textContent =
      "No included series in this config.";
    return;
  }
  for (const card of page.cards) {
    root.appendChild(renderCard(card));
  }
}

applyConfiguredNav().then((enabled) => {
  if (!enabled) return;
  loadPage().catch((err) => {
    document.getElementById("error").textContent = String(err);
  });
});
