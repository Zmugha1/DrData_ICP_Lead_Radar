const state = {
  offset: 0,
  limit: 100,
  totalAfter: 0,
  countyFips: "",
};

// Failures go in #command-error via nav.js showCommandFailure.
// An empty successful page says no rows matched and shows counts of zero.

let loadingTick = null;

function startProgress(label) {
  stopProgress();
  const started = Date.now();
  const counts = document.getElementById("counts");
  const tick = () => {
    const seconds = Math.floor((Date.now() - started) / 1000);
    counts.textContent =
      seconds === 0 ? label + "…" : label + " (" + seconds + "s)";
  };
  tick();
  loadingTick = setInterval(tick, 250);
}

function stopProgress() {
  if (loadingTick != null) {
    clearInterval(loadingTick);
    loadingTick = null;
  }
}

function fillNone(select) {
  select.innerHTML = "";
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "none";
  none.disabled = true;
  select.appendChild(none);
}

function unknownCell(value, isNumericZeroAllowed) {
  const td = document.createElement("td");
  const missing =
    value === null ||
    value === undefined ||
    (typeof value === "string" && value.trim() === "");
  if (missing) {
    const mark = document.createElement("span");
    mark.className = "unknown";
    mark.textContent = "unknown";
    td.appendChild(mark);
    return td;
  }
  if (isNumericZeroAllowed && value === 0) {
    td.textContent = "0";
    return td;
  }
  td.textContent = String(value);
  return td;
}

function renderCaveat(caveat) {
  const el = document.getElementById("caveat");
  el.hidden = false;
  const found =
    caveat &&
    (caveat.assignment_found === true || caveat.assignmentFound === true);
  const text = caveat
    ? caveat.text !== undefined
      ? caveat.text
      : caveat.Text
    : null;
  if (!found) {
    el.textContent =
      "No zone assignment was found for prospect ranking by likelihood to buy.";
    return;
  }
  if (text) {
    el.textContent = text;
    return;
  }
  el.textContent = "Zone assignment found with no display caveat.";
}

function fillSelect(select, options, valueKey, labelFn, includeAll) {
  if (!options || options.length === 0) {
    fillNone(select);
    return;
  }
  select.innerHTML = "";
  if (includeAll) {
    const all = document.createElement("option");
    all.value = "";
    all.textContent = "All";
    select.appendChild(all);
  }
  for (const item of options) {
    const option = document.createElement("option");
    if (typeof item === "string") {
      option.value = item;
      option.textContent = item;
    } else {
      option.value = item[valueKey];
      option.textContent = labelFn(item);
    }
    select.appendChild(option);
  }
}

function fillBandSelect(select, bands) {
  if (!bands || bands.length === 0) {
    fillNone(select);
    return;
  }
  select.innerHTML = "";
  const all = document.createElement("option");
  all.value = "";
  all.textContent = "All";
  select.appendChild(all);
  for (const band of bands) {
    const option = document.createElement("option");
    option.dataset.kind = band.kind;
    if (band.kind === "band") {
      option.dataset.label = band.label;
      option.textContent = band.label;
    } else if (band.kind === "out_of_bands") {
      option.textContent = "Out of bands";
    } else if (band.kind === "unknown") {
      option.textContent = "Unknown";
    } else {
      option.textContent = band.kind;
    }
    select.appendChild(option);
  }
}

function selectedBand() {
  const select = document.getElementById("band");
  const option = select.selectedOptions[0];
  if (!option || !option.dataset.kind) return null;
  if (option.dataset.kind === "band") {
    return { kind: "band", label: option.dataset.label };
  }
  if (option.dataset.kind === "out_of_bands") {
    return { kind: "out_of_bands" };
  }
  if (option.dataset.kind === "unknown") {
    return { kind: "unknown" };
  }
  return null;
}

function numberOrNull(id) {
  const raw = document.getElementById(id).value;
  if (raw === "" || raw === null) return null;
  const value = Number(raw);
  if (!Number.isFinite(value)) return null;
  return value;
}

function currentQuery() {
  const county = document.getElementById("county").value;
  const municipality = document.getElementById("municipality").value;
  const ownerType = document.getElementById("owner-type").value;
  const name = document.getElementById("name-contains").value;
  return {
    county_fips: county || null,
    municipality: municipality || null,
    owner_type: ownerType || null,
    band: selectedBand(),
    min_parcels: numberOrNull("min-parcels"),
    min_improvement_value_zero: numberOrNull("min-zero"),
    name_contains: name.trim() ? name : null,
    offset: state.offset,
    limit: state.limit,
    apply_default_owner_type_exclusions: document.getElementById("hide-government")
      .checked,
  };
}

function activeFilterText(query) {
  const parts = [];
  if (query.county_fips) parts.push("county_fips=" + query.county_fips);
  if (query.municipality) parts.push("municipality=" + query.municipality);
  if (query.owner_type) parts.push("owner_type=" + query.owner_type);
  if (query.apply_default_owner_type_exclusions !== false && !query.owner_type) {
    parts.push("government, tribal, and placeholder owners are hidden by default");
  }
  if (query.band) {
    if (query.band.kind === "band") parts.push("band=" + query.band.label);
    else parts.push("band_state=" + query.band.kind);
  }
  if (query.min_parcels != null) parts.push("min_parcels=" + query.min_parcels);
  if (query.min_improvement_value_zero != null) {
    parts.push("min_improvement_value_zero=" + query.min_improvement_value_zero);
  }
  if (query.name_contains) parts.push("name_contains=" + query.name_contains);
  if (parts.length === 0) return "Active filters: none";
  return "Active filters: " + parts.join("; ");
}

function renderBand(band) {
  if (!band) {
    return { label: null, favourability: null };
  }
  if (band.kind === "band") {
    return { label: band.label, favourability: band.favourability };
  }
  if (band.kind === "out_of_bands") {
    return {
      label: null,
      favourability: null,
      state: "outside declared bands; value " + band.value,
    };
  }
  return { label: null, favourability: null, state: "unknown" };
}

function renderGroups(page) {
  const totalBefore =
    page.total_before_filter !== undefined
      ? page.total_before_filter
      : page.totalBeforeFilter;
  const totalAfter =
    page.total_after_filter !== undefined
      ? page.total_after_filter
      : page.totalAfterFilter;
  renderCaveat(page.caveat);
  document.getElementById("active-filters").textContent = activeFilterText(
    currentQuery()
  );
  document.getElementById("counts").textContent =
    "Count before filtering: " +
    totalBefore +
    ". Count after filtering: " +
    totalAfter +
    ".";
  document.getElementById("predicate").textContent = page.predicate_chain;
  state.totalAfter = totalAfter;
  state.offset = page.offset;
  state.limit = page.limit;

  const start = totalAfter === 0 ? 0 : page.offset + 1;
  const end = Math.min(page.offset + page.rows.length, totalAfter);
  document.getElementById("page-controls").textContent =
    "Showing " +
    start +
    " to " +
    end +
    " of " +
    totalAfter +
    ".";

  const emptyNote = document.getElementById("empty-result");
  if (totalAfter === 0) {
    emptyNote.textContent = "no rows matched";
  } else {
    emptyNote.textContent = "";
  }

  const body = document.querySelector("#groups tbody");
  body.innerHTML = "";
  for (const row of page.rows) {
    const tr = document.createElement("tr");
    tr.tabIndex = 0;
    tr.dataset.groupKey = row.group_key;
    const band = renderBand(row.band);
    tr.appendChild(document.createElement("td")).textContent = row.display_name;
    tr.appendChild(document.createElement("td")).textContent = row.group_key;
    tr.appendChild(unknownCell(row.member_name_count, true));
    tr.appendChild(unknownCell(row.total_parcels, true));
    tr.appendChild(unknownCell(row.improvement_value_zero_count, true));
    tr.appendChild(unknownCell(row.municipality_count, true));
    tr.appendChild(unknownCell(row.owner_type, false));
    const mixed = document.createElement("td");
    mixed.textContent = row.mixed_owner_type ? "yes" : "no";
    tr.appendChild(mixed);
    if (band.label) {
      tr.appendChild(document.createElement("td")).textContent = band.label;
    } else {
      const td = document.createElement("td");
      const mark = document.createElement("span");
      mark.className = "unknown";
      mark.textContent = band.state || "unknown";
      td.appendChild(mark);
      tr.appendChild(td);
    }
    tr.appendChild(unknownCell(band.favourability, false));
    tr.appendChild(unknownCell(row.value_as_of, false));
    tr.appendChild(
      outcomeRowCell(row, {
        subject_type: "owner_group",
        subject_key: row.group_key,
        county_fips: state.countyFips || null,
      })
    );
    tr.addEventListener("click", (event) => {
      if (event.target && event.target.tagName === "BUTTON") return;
      showParcels(row.group_key, row.display_name);
    });
    tr.addEventListener("keydown", (event) => {
      if (event.target && event.target.tagName === "BUTTON") return;
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        showParcels(row.group_key, row.display_name);
      }
    });
    body.appendChild(tr);
  }
}

async function showParcels(groupKey, displayName) {
  const panel = document.getElementById("parcels-panel");
  panel.hidden = false;
  document.getElementById("parcels-heading").textContent =
    "Parcels for " + displayName;
  const body = document.querySelector("#parcels tbody");
  body.innerHTML = "";
  let rows;
  try {
    rows = await invoke("owner_group_parcels", { groupKey: groupKey });
  } catch (err) {
    showCommandFailure("owner_group_parcels", err);
    return;
  }
  for (const row of rows) {
    const tr = document.createElement("tr");
    tr.appendChild(unknownCell(row.taxkey, false));
    tr.appendChild(unknownCell(row.site_address, false));
    tr.appendChild(unknownCell(row.municipality, false));
    tr.appendChild(unknownCell(row.owner_name, false));
    tr.appendChild(unknownCell(row.assessed_value, true));
    tr.appendChild(unknownCell(row.improvement_value, true));
    tr.appendChild(unknownCell(row.land_value, true));
    tr.appendChild(unknownCell(row.prop_class, false));
    tr.appendChild(unknownCell(row.value_source, false));
    tr.appendChild(unknownCell(row.value_as_of, false));
    body.appendChild(tr);
  }
}

async function loadPage() {
  document.getElementById("error").textContent = "";
  startProgress("Loading owner groups");
  let page;
  try {
    page = await invoke("list_owner_groups", { query: currentQuery() });
  } catch (err) {
    stopProgress();
    showCommandFailure("list_owner_groups", err);
    document.getElementById("empty-result").textContent = "";
    document.getElementById("counts").textContent = "";
    throw err;
  }
  if (!page || !Array.isArray(page.rows)) {
    stopProgress();
    showCommandFailure(
      "list_owner_groups",
      "response had no rows array: " + JSON.stringify(page)
    );
    document.getElementById("counts").textContent = "";
    return;
  }
  stopProgress();
  clearCommandFailure();
  renderGroups(page);
}

async function setup() {
  startProgress("Loading filters");
  let options;
  try {
    options = await invoke("owner_group_filter_options");
  } catch (err) {
    stopProgress();
    showCommandFailure("owner_group_filter_options", err);
    fillNone(document.getElementById("county"));
    fillNone(document.getElementById("municipality"));
    fillNone(document.getElementById("owner-type"));
    fillNone(document.getElementById("band"));
    document.getElementById("counts").textContent = "";
    return;
  }
  document.getElementById("page-title").textContent =
    "Owner groups · " + options.vertical_display_name;
  state.countyFips = options.county_fips || "";
  document.getElementById("config-line").textContent =
    options.county_name +
    " (" +
    options.county_fips +
    "). Config version " +
    options.config_version +
    ".";
  renderCaveat(options.caveat);
  fillSelect(
    document.getElementById("county"),
    options.counties,
    "county_fips",
    (item) => item.county_name + " (" + item.county_fips + ")",
    true
  );
  fillSelect(
    document.getElementById("municipality"),
    options.municipalities,
    "muni_code",
    (item) => (item.muni_name ? item.muni_name + " (" + item.muni_code + ")" : item.muni_code),
    true
  );
  fillSelect(document.getElementById("owner-type"), options.owner_types, null, null, true);
  fillBandSelect(document.getElementById("band"), options.bands);
  const notice = document.getElementById("default-filter-notice");
  const excluded = (options.default_excluded_owner_types || []).join(", ");
  notice.textContent =
    "Government, tribal, and placeholder owners are hidden by default (" +
    (excluded || "government, placeholder, tribal") +
    "). Uncheck Hide government, tribal, and placeholder owners, or choose that owner type in the filter, to see those rows.";
  document.getElementById("hide-government").checked = true;
  try {
    await loadPage();
  } catch (err) {
    showCommandFailure("list_owner_groups", err);
    document.getElementById("empty-result").textContent = "";
    document.getElementById("counts").textContent = "";
  }
}

document.getElementById("filters").addEventListener("submit", (event) => {
  event.preventDefault();
  state.offset = 0;
  loadPage().catch((err) => {
    showCommandFailure("list_owner_groups", err);
  });
});

document.getElementById("hide-government").addEventListener("change", () => {
  state.offset = 0;
  loadPage().catch((err) => {
    showCommandFailure("list_owner_groups", err);
  });
});

document.getElementById("export").addEventListener("click", () => {
  invoke("export_owner_groups", { query: currentQuery() })
    .then((result) => {
      clearCommandFailure();
      document.getElementById("error").textContent =
        "Exported " + result.rows_exported + " rows to " + result.csv_path;
    })
    .catch((err) => {
      showCommandFailure("export_owner_groups", err);
    });
});

const pager = document.createElement("div");
pager.className = "card-actions";
const prev = document.createElement("button");
prev.type = "button";
prev.className = "btn-secondary";
prev.textContent = "Previous page";
prev.addEventListener("click", () => {
  state.offset = Math.max(0, state.offset - state.limit);
  loadPage().catch((err) => {
    showCommandFailure("list_owner_groups", err);
  });
});
const next = document.createElement("button");
next.type = "button";
next.className = "btn-secondary";
next.textContent = "Next page";
next.addEventListener("click", () => {
  if (state.offset + state.limit < state.totalAfter) {
    state.offset += state.limit;
    loadPage().catch((err) => {
      showCommandFailure("list_owner_groups", err);
    });
  }
});
pager.appendChild(prev);
pager.appendChild(document.createTextNode(" "));
pager.appendChild(next);
document.getElementById("page-controls").after(pager);

applyConfiguredNav().then((enabled) => {
  if (!enabled) return;
  setup().catch((err) => {
    showCommandFailure("owner_group_filter_options", err);
  });
});

document.addEventListener("outcomes-changed", () => {
  loadPage().catch((err) => {
    showCommandFailure("list_owner_groups", err);
  });
});
