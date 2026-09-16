const state = {
  offset: 0,
  limit: 100,
  totalAfter: 0,
};

function fillNone(select) {
  select.innerHTML = "";
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "none";
  none.disabled = true;
  select.appendChild(none);
}

function fillSelect(select, options) {
  if (!options || options.length === 0) {
    fillNone(select);
    return;
  }
  select.innerHTML = "";
  const all = document.createElement("option");
  all.value = "";
  all.textContent = "All";
  select.appendChild(all);
  for (const item of options) {
    const option = document.createElement("option");
    option.value = item;
    option.textContent = item;
    select.appendChild(option);
  }
}

function optionalTextCell(value) {
  const td = document.createElement("td");
  const missing =
    value === null ||
    value === undefined ||
    (typeof value === "string" && value.trim() === "");
  if (missing) {
    td.textContent = "";
    return td;
  }
  td.textContent = String(value);
  return td;
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

function emptyOrNull(id) {
  const raw = document.getElementById(id).value;
  if (raw === "" || raw === null) return null;
  return raw;
}

function currentQuery() {
  const name = document.getElementById("name-contains").value;
  return {
    county: emptyOrNull("county"),
    city: emptyOrNull("city"),
    conveyance_text: emptyOrNull("conveyance"),
    recorded_date_from: emptyOrNull("recorded-from"),
    recorded_date_to: emptyOrNull("recorded-to"),
    last_update_date_from: emptyOrNull("updated-from"),
    last_update_date_to: emptyOrNull("updated-to"),
    name_contains: name.trim() ? name : null,
    offset: state.offset,
    limit: state.limit,
  };
}

function countyLabel(row) {
  if (row.county_unmatched) return "unmatched";
  if (row.county) return row.county;
  return null;
}

function assignmentCountyFips(row) {
  if (row.county_unmatched) return null;
  if (row.county === "waukesha") return "55133";
  if (row.county === "milwaukee") return "55079";
  return null;
}

function renderPage(page) {
  document.getElementById("active-filters").textContent = page.active_filters_text;
  document.getElementById("counts").textContent =
    "Count before filtering: " +
    page.total_before_filter +
    ". Count after filtering: " +
    page.total_after_filter +
    ".";
  document.getElementById("parcel-match").textContent = page.parcel_match.text;
  const stale = document.getElementById("staleness");
  stale.textContent = page.staleness.text;
  stale.hidden = false;

  state.totalAfter = page.total_after_filter;
  state.offset = page.offset;
  state.limit = page.limit;

  const start = page.total_after_filter === 0 ? 0 : page.offset + 1;
  const end = Math.min(page.offset + page.rows.length, page.total_after_filter);
  document.getElementById("page-controls").textContent =
    "Showing " + start + " to " + end + " of " + page.total_after_filter + ".";

  const emptyNote = document.getElementById("empty-result");
  emptyNote.textContent = page.total_after_filter === 0 ? "no rows matched" : "";

  const body = document.querySelector("#assignments tbody");
  body.innerHTML = "";
  for (const row of page.rows) {
    const tr = document.createElement("tr");
    tr.tabIndex = 0;
    tr.appendChild(unknownCell(row.company_name, false));
    tr.appendChild(unknownCell(row.city, false));
    tr.appendChild(unknownCell(countyLabel(row), false));
    tr.appendChild(unknownCell(row.address_1, false));
    tr.appendChild(optionalTextCell(row.address_2));
    tr.appendChild(unknownCell(row.postcode, false));
    tr.appendChild(unknownCell(row.conveyance_text, false));
    tr.appendChild(unknownCell(row.recorded_date, false));
    tr.appendChild(unknownCell(row.last_update_date, false));
    tr.appendChild(unknownCell(row.reel_no, false));
    tr.appendChild(unknownCell(row.frame_no, false));
    tr.appendChild(unknownCell(row.patent_count, true));
    tr.appendChild(unknownCell(row.assignor_names, false));
    const attorney = document.createElement("td");
    const attorneyName =
      row.filing_attorney_name === null ||
      row.filing_attorney_name === undefined ||
      String(row.filing_attorney_name).trim() === "";
    if (attorneyName) {
      const mark = document.createElement("span");
      mark.className = "unknown";
      mark.textContent = "unknown";
      attorney.appendChild(mark);
    } else {
      attorney.textContent = row.filing_attorney_name;
    }
    tr.appendChild(attorney);
    const parcelTd = document.createElement("td");
    if (!row.parcel) {
      parcelTd.textContent = "";
    } else {
      const link = document.createElement("a");
      link.href = "groups.html";
      link.textContent =
        row.parcel.parcel_count +
        " parcels, " +
        row.parcel.improvement_value_zero_count +
        " improvement-value-zero";
      parcelTd.appendChild(link);
    }
    tr.appendChild(parcelTd);
    tr.appendChild(
      outcomeRowCell(row, {
        subject_type: "assignment_assignee",
        subject_key: row.reel_no + "/" + row.frame_no + "/" + String(row.sequence),
        county_fips: assignmentCountyFips(row),
      })
    );
    tr.addEventListener("click", (event) => {
      if (event.target && (event.target.tagName === "A" || event.target.tagName === "BUTTON")) {
        return;
      }
      showDetail(row.reel_no, row.frame_no);
    });
    tr.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        showDetail(row.reel_no, row.frame_no);
      }
    });
    body.appendChild(tr);
  }
}

async function showDetail(reelNo, frameNo) {
  const panel = document.getElementById("detail-panel");
  panel.hidden = false;
  document.getElementById("detail-heading").textContent =
    "Assignment reel " + reelNo + " frame " + frameNo;
  let detail;
  try {
    detail = await invoke("assignment_detail", {
      reel_no: reelNo,
      frame_no: frameNo,
    });
  } catch (err) {
    showCommandFailure("assignment_detail", err);
    return;
  }
  document.getElementById("detail-meta").textContent =
    "Recorded date: " +
    (detail.recorded_date || "unknown") +
    ". Last update date: " +
    (detail.last_update_date || "unknown") +
    ". Conveyance: " +
    (detail.conveyance_text || "unknown") +
    ".";
  document.getElementById("detail-attorney").textContent =
    "Filing attorney (correspondent, not the business): " +
    (detail.filing_attorney_name || "unknown") +
    (detail.filing_attorney_address ? ". " + detail.filing_attorney_address : ".");

  const assignees = document.querySelector("#detail-assignees tbody");
  assignees.innerHTML = "";
  for (const row of detail.assignees) {
    const tr = document.createElement("tr");
    tr.appendChild(unknownCell(row.name, false));
    tr.appendChild(unknownCell(row.city, false));
    tr.appendChild(unknownCell(countyLabel(row), false));
    tr.appendChild(unknownCell(row.address_1, false));
    tr.appendChild(optionalTextCell(row.address_2));
    tr.appendChild(unknownCell(row.postcode, false));
    assignees.appendChild(tr);
  }

  const assignors = document.querySelector("#detail-assignors tbody");
  assignors.innerHTML = "";
  for (const row of detail.assignors) {
    const tr = document.createElement("tr");
    tr.appendChild(unknownCell(row.name, false));
    tr.appendChild(unknownCell(row.execution_date, false));
    assignors.appendChild(tr);
  }

  const properties = document.querySelector("#detail-properties tbody");
  properties.innerHTML = "";
  for (const row of detail.properties) {
    const tr = document.createElement("tr");
    tr.appendChild(unknownCell(row.country, false));
    tr.appendChild(unknownCell(row.doc_number, false));
    tr.appendChild(unknownCell(row.kind, false));
    tr.appendChild(unknownCell(row.invention_title, false));
    properties.appendChild(tr);
  }
}

async function loadFolderStatus() {
  const status = await invoke("assignment_folder_status");
  document.getElementById("folder-status").textContent = status.message;
}

async function loadPage() {
  document.getElementById("error").textContent = "";
  clearCommandFailure();
  const page = await invoke("list_assignments", { query: currentQuery() });
  renderPage(page);
}

async function setup() {
  document.getElementById("counts").textContent = "Loading…";
  let options;
  try {
    options = await invoke("assignment_filter_options");
  } catch (err) {
    showCommandFailure("assignment_filter_options", err);
    fillNone(document.getElementById("county"));
    fillNone(document.getElementById("city"));
    fillNone(document.getElementById("conveyance"));
    document.getElementById("counts").textContent = "";
    return;
  }
  document.getElementById("page-title").textContent =
    "Assignments · " + options.vertical_display_name;
  document.getElementById("config-line").textContent =
    "Config version " + options.config_version + ".";
  fillSelect(document.getElementById("county"), options.counties);
  fillSelect(document.getElementById("city"), options.cities);
  fillSelect(document.getElementById("conveyance"), options.conveyance_texts);
  try {
    await loadFolderStatus();
  } catch (err) {
    showCommandFailure("assignment_folder_status", err);
  }
  try {
    await loadPage();
  } catch (err) {
    showCommandFailure("list_assignments", err);
    document.getElementById("empty-result").textContent = "";
    document.getElementById("counts").textContent = "";
  }
}

document.getElementById("filters").addEventListener("submit", (event) => {
  event.preventDefault();
  state.offset = 0;
  loadPage().catch((err) => {
    showCommandFailure("list_assignments", err);
  });
});

document.getElementById("refresh").addEventListener("click", () => {
  document.getElementById("refresh-result").textContent = "Refreshing…";
  invoke("refresh_assignments")
    .then(async (result) => {
      clearCommandFailure();
      document.getElementById("folder-status").textContent = result.folder_status.message;
      document.getElementById("refresh-result").textContent = result.summary;
      const options = await invoke("assignment_filter_options");
      fillSelect(document.getElementById("county"), options.counties);
      fillSelect(document.getElementById("city"), options.cities);
      fillSelect(document.getElementById("conveyance"), options.conveyance_texts);
      state.offset = 0;
      await loadPage();
    })
    .catch((err) => {
      showCommandFailure("refresh_assignments", err);
      document.getElementById("refresh-result").textContent = "";
    });
});

document.getElementById("export").addEventListener("click", () => {
  invoke("pick_assignment_export_path")
    .then((path) => {
      if (!path) return;
      return invoke("export_assignments", { query: currentQuery(), path }).then(
        (result) => {
          clearCommandFailure();
          document.getElementById("error").textContent =
            "Exported " +
            result.rows_exported +
            " rows to " +
            result.csv_path +
            ". Provenance: " +
            result.provenance_path;
        }
      );
    })
    .catch((err) => {
      showCommandFailure("export_assignments", err);
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
    showCommandFailure("list_assignments", err);
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
      showCommandFailure("list_assignments", err);
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
    showCommandFailure("assignment_filter_options", err);
  });
});

document.addEventListener("outcomes-changed", () => {
  loadPage().catch((err) => {
    showCommandFailure("list_assignments", err);
  });
});
