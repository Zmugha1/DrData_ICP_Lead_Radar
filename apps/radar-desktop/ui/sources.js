// One record per sources.toml entry, grouped by status. This is not a
// page builder. No fetch button lives here.

function text(tag, className, value) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  el.textContent = value;
  return el;
}

function yn(value) {
  return value ? "true" : "false";
}

function renderSource(row) {
  const section = document.createElement("section");
  section.className = "card card-lg";
  section.appendChild(text("p", "card-kicker", row.source_id));
  section.appendChild(text("h2", "card-title", row.display_name));
  section.appendChild(
    text(
      "p",
      "slate",
      "family " +
        row.family +
        ". kind " +
        row.kind +
        ". status " +
        row.status +
        "."
    )
  );
  const cadence =
    row.cadence_days === null || row.cadence_days === undefined
      ? "none"
      : String(row.cadence_days) + " days";
  const lag =
    row.publication_lag_days === null || row.publication_lag_days === undefined
      ? "none"
      : String(row.publication_lag_days) + " days";
  section.appendChild(
    text(
      "p",
      "slate",
      "cadence " +
        cadence +
        ". stated publication lag " +
        lag +
        ", measured=" +
        yn(row.lag_measured) +
        ". url_verified=" +
        yn(row.url_verified) +
        ". requires_key=" +
        yn(row.requires_key) +
        "."
    )
  );
  if (row.attribution) {
    section.appendChild(text("p", "slate", "attribution " + row.attribution));
  }
  section.appendChild(text("p", "card-body", "Last successful load: " + row.fetch_text + "."));
  section.appendChild(text("p", "slate", row.overdue_text));
  section.appendChild(
    text(
      "p",
      "slate",
      String(row.signal_count) +
        " signals name this source. " +
        String(row.detection_built_count) +
        " have detection built."
    )
  );
  if (row.live_without_detection) {
    section.appendChild(
      text(
        "p",
        "slate",
        "Live source with no detection built produces nothing."
      )
    );
  }
  if (row.folder_path || row.download_page) {
    let manual = "Manual upload.";
    if (row.folder_path) manual += " Folder " + row.folder_path + ".";
    if (row.download_page) manual += " Download page " + row.download_page + ".";
    section.appendChild(text("p", "card-body", manual));
  }
  if (row.notes) {
    const label = row.status === "Refused" ? "Reason: " : "Notes: ";
    section.appendChild(text("p", "slate", label + row.notes));
  }
  return section;
}

function renderGroup(group) {
  const wrap = document.createElement("div");
  wrap.appendChild(text("h2", "card-title", group.status));
  if (!group.rows || group.rows.length === 0) {
    wrap.appendChild(text("p", "slate", "None."));
    return wrap;
  }
  for (const row of group.rows) {
    wrap.appendChild(renderSource(row));
  }
  return wrap;
}

async function loadPage() {
  clearCommandFailure();
  document.getElementById("error").textContent = "";
  const root = document.getElementById("sections");
  root.innerHTML = "";
  let page;
  try {
    page = await invoke("sources_page");
  } catch (err) {
    showCommandFailure("sources_page", err);
    return;
  }
  document.getElementById("page-title").textContent =
    "Sources · " + page.vertical_display_name;
  document.getElementById("config-line").textContent =
    page.county_name +
    " (" +
    page.county_fips +
    "). Config version " +
    page.config_version +
    ".";
  if (!page.groups || page.groups.length === 0) {
    root.appendChild(text("p", "slate", "No sources in sources.toml."));
    return;
  }
  for (const group of page.groups) {
    root.appendChild(renderGroup(group));
  }
}

applyConfiguredNav().then((enabled) => {
  if (!enabled) return;
  loadPage().catch((err) => {
    document.getElementById("error").textContent = String(err);
  });
});
