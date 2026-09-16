// Three sections are fixed: geographies, attributes, publication lag.
// coverage.toml and the store fill those sections. This is not a page builder.

function text(tag, className, value) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  el.textContent = value;
  return el;
}

function originMark(origin) {
  return origin ? " (" + origin + ")" : "";
}

function cell(value) {
  const td = document.createElement("td");
  if (value === null || value === undefined || value === "") {
    const mark = document.createElement("span");
    mark.className = "unknown";
    mark.textContent = "unknown";
    td.appendChild(mark);
    return td;
  }
  td.textContent = String(value);
  return td;
}

function headRow(labels) {
  const tr = document.createElement("tr");
  for (const label of labels) {
    const th = document.createElement("th");
    th.textContent = label;
    tr.appendChild(th);
  }
  return tr;
}

function table(labels, fillBody) {
  const wrap = document.createElement("div");
  wrap.className = "table-wrap";
  const tableEl = document.createElement("table");
  const thead = document.createElement("thead");
  thead.appendChild(headRow(labels));
  tableEl.appendChild(thead);
  const tbody = document.createElement("tbody");
  fillBody(tbody);
  tableEl.appendChild(tbody);
  wrap.appendChild(tableEl);
  return wrap;
}

function sectionCard(kicker, title, bodyFn) {
  const section = document.createElement("section");
  section.className = "card card-lg";
  section.appendChild(text("p", "card-kicker", kicker));
  section.appendChild(text("h2", "card-title", title));
  bodyFn(section);
  return section;
}

function formatPercent(value) {
  if (value === null || value === undefined) return null;
  return value.toFixed(1) + "%";
}

function renderGeographies(page) {
  return sectionCard("Geographies", "Geographies", (section) => {
    if (!page.geographies || page.geographies.length === 0) {
      section.appendChild(text("p", "slate", "No geographies in coverage.toml."));
      return;
    }
    section.appendChild(
      table(
        ["Key", "Level", "Note", "Rows"],
        (tbody) => {
          for (const row of page.geographies) {
            const tr = document.createElement("tr");
            tr.appendChild(cell(row.key));
            tr.appendChild(
              cell(
                (row.level || "not stated") + originMark(row.level_origin)
              )
            );
            const note = document.createElement("td");
            note.textContent =
              (row.note || "") +
              (row.note_origin ? originMark(row.note_origin) : "");
            tr.appendChild(note);
            let rowsText = String(row.measured_row_count) + originMark(row.row_count_origin);
            if (row.measured_classed_count !== null && row.measured_classed_count !== undefined) {
              rowsText +=
                ". classed " +
                String(row.measured_classed_count) +
                originMark(row.row_count_origin);
            }
            tr.appendChild(cell(rowsText));
            tbody.appendChild(tr);
            if (row.historic_municipalities && row.historic_municipalities.length > 0) {
              const extra = document.createElement("tr");
              const td = document.createElement("td");
              td.colSpan = 4;
              const parts = row.historic_municipalities.map((muni) => {
                let bit = muni.name + " " + String(muni.row_count) + " rows";
                if (muni.value_as_of) bit += ", current to " + muni.value_as_of;
                return bit + originMark(muni.origin);
              });
              td.textContent =
                "Previous year-end roll: " + parts.join(". ") + ".";
              extra.appendChild(td);
              tbody.appendChild(extra);
            }
          }
        }
      )
    );
  });
}

function renderAttributes(page) {
  return sectionCard("Attributes", "Attributes", (section) => {
    section.appendChild(
      text(
        "p",
        "slate",
        String(page.three_value_null_count) +
          " rows carry null assessed, improvement and land value" +
          originMark(page.three_value_null_origin) +
          "."
      )
    );
    if (page.value_null_municipalities && page.value_null_municipalities.length > 0) {
      const names = page.value_null_municipalities
        .map(
          (muni) =>
            muni.name +
            " " +
            String(muni.row_count) +
            " rows" +
            originMark(muni.origin)
        )
        .join(". ");
      section.appendChild(
        text(
          "p",
          "slate",
          "Municipalities with no assessed, improvement or land value on any row: " +
            names +
            "."
        )
      );
    }
    if (!page.attributes || page.attributes.length === 0) {
      section.appendChild(text("p", "slate", "No attributes in coverage.toml."));
      return;
    }
    section.appendChild(
      table(
        ["Key", "Stated level", "Note", "Nulls"],
        (tbody) => {
          for (const row of page.attributes) {
            const tr = document.createElement("tr");
            tr.appendChild(cell(row.key));
            const stated = row.stated_level
              ? row.stated_level + originMark(row.level_origin)
              : "not stated" + originMark(row.level_origin);
            tr.appendChild(cell(stated));
            let note = row.note || "";
            if (row.note_origin) note += originMark(row.note_origin);
            if (row.not_in_store) {
              if (note) note += " ";
              note += "not in the store (measured).";
            }
            tr.appendChild(cell(note || null));
            let nulls = "";
            if (row.measured_null_count !== null && row.measured_null_count !== undefined) {
              nulls =
                String(row.measured_null_count) +
                " of " +
                String(row.measured_row_count) +
                originMark(row.measurement_origin);
              const pct = formatPercent(row.measured_null_percent);
              if (pct) nulls += ", " + pct;
            } else {
              nulls = "not measured";
            }
            if (row.disagreement_text) {
              nulls += ". " + row.disagreement_text;
            }
            tr.appendChild(cell(nulls));
            tbody.appendChild(tr);
          }
        }
      )
    );
  });
}

function renderLags(page) {
  return sectionCard("Publication lag", "Publication lag", (section) => {
    if (!page.publication_lags || page.publication_lags.length === 0) {
      section.appendChild(
        text("p", "slate", "No publication lags in coverage.toml.")
      );
      return;
    }
    section.appendChild(
      table(
        ["Source", "Stated lag", "Newest data", "Age"],
        (tbody) => {
          for (const row of page.publication_lags) {
            const tr = document.createElement("tr");
            let name = row.display_name || row.source_id;
            if (row.series_id) name += " " + row.series_id;
            tr.appendChild(cell(name));
            let stated = "not stated" + originMark(row.stated_origin);
            if (row.stated_lag_days !== null && row.stated_lag_days !== undefined) {
              stated =
                String(row.stated_lag_days) +
                " days, measured=" +
                String(row.stated_measured) +
                originMark(row.stated_origin);
            }
            tr.appendChild(cell(stated));
            tr.appendChild(cell(row.newest_date));
            tr.appendChild(cell(row.age_text));
            tbody.appendChild(tr);
          }
        }
      )
    );
  });
}

async function loadPage() {
  clearCommandFailure();
  document.getElementById("error").textContent = "";
  const root = document.getElementById("sections");
  root.innerHTML = "";
  let page;
  try {
    page = await invoke("coverage_page");
  } catch (err) {
    showCommandFailure("coverage_page", err);
    return;
  }
  document.getElementById("page-title").textContent =
    "Coverage · " + page.vertical_display_name;
  document.getElementById("config-line").textContent =
    page.county_name +
    " (" +
    page.county_fips +
    "). Config version " +
    page.config_version +
    ".";
  document.getElementById("framing").textContent = page.framing;
  root.appendChild(renderGeographies(page));
  root.appendChild(renderAttributes(page));
  root.appendChild(renderLags(page));
}

applyConfiguredNav().then((enabled) => {
  if (!enabled) return;
  loadPage().catch((err) => {
    document.getElementById("error").textContent = String(err);
  });
});
