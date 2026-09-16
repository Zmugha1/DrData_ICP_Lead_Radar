const OUTCOME_ACTIONS = [
  "contacted",
  "met",
  "proposed",
  "closed_won",
  "closed_lost",
  "not_a_fit",
];

const outcomePanelState = {
  subject_type: null,
  subject_key: null,
  county_fips: null,
};

function fillOutcomeActions(select) {
  select.innerHTML = "";
  for (const action of OUTCOME_ACTIONS) {
    const option = document.createElement("option");
    option.value = action;
    option.textContent = action;
    select.appendChild(option);
  }
}

function outcomeRowCell(row, spec) {
  const td = document.createElement("td");
  if (row.latest_outcome) {
    td.appendChild(
      document.createTextNode(
        row.latest_outcome.action + " " + row.latest_outcome.occurred_on
      )
    );
    td.appendChild(document.createTextNode(" "));
  }
  const button = document.createElement("button");
  button.type = "button";
  button.className = "btn-secondary";
  button.textContent = "Outcomes";
  button.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    openOutcomePanel(spec);
  });
  td.appendChild(button);
  return td;
}

function renderOutcomeHistory(page) {
  const body = document.querySelector("#outcome-history tbody");
  body.innerHTML = "";
  for (const row of page.outcomes) {
    const tr = document.createElement("tr");
    const action = document.createElement("td");
    action.textContent = row.action;
    const date = document.createElement("td");
    date.textContent = row.occurred_on;
    const reason = document.createElement("td");
    reason.textContent =
      row.reason === null || row.reason === undefined ? "" : String(row.reason);
    const fingerprint = document.createElement("td");
    fingerprint.textContent = row.config_fingerprint;
    tr.appendChild(action);
    tr.appendChild(date);
    tr.appendChild(reason);
    tr.appendChild(fingerprint);
    body.appendChild(tr);
  }
}

async function loadOutcomeHistory() {
  const page = await invoke("outcome_history", {
    subjectType: outcomePanelState.subject_type,
    subjectKey: outcomePanelState.subject_key,
  });
  renderOutcomeHistory(page);
  const date = document.getElementById("outcome-date");
  if (!date.value) {
    date.value = page.today;
  }
  return page;
}

async function openOutcomePanel(spec) {
  outcomePanelState.subject_type = spec.subject_type;
  outcomePanelState.subject_key = spec.subject_key;
  outcomePanelState.county_fips = spec.county_fips || null;
  const panel = document.getElementById("outcome-panel");
  panel.hidden = false;
  document.getElementById("outcome-heading").textContent = "Outcomes";
  document.getElementById("outcome-subject").textContent =
    spec.subject_type + " " + spec.subject_key;
  document.getElementById("outcome-error").textContent = "";
  document.getElementById("outcome-reason").value = "";
  fillOutcomeActions(document.getElementById("outcome-action"));
  document.getElementById("outcome-date").value = "";
  try {
    await loadOutcomeHistory();
  } catch (err) {
    document.getElementById("outcome-error").textContent = String(err);
  }
}

function initOutcomePanel() {
  const form = document.getElementById("outcome-form");
  if (!form || form.dataset.bound === "1") return;
  form.dataset.bound = "1";
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const action = document.getElementById("outcome-action").value;
    const occurredOn = document.getElementById("outcome-date").value;
    const reasonRaw = document.getElementById("outcome-reason").value;
    const reason = reasonRaw.trim() === "" ? null : reasonRaw;
    invoke("record_outcome", {
      request: {
        subject_type: outcomePanelState.subject_type,
        subject_key: outcomePanelState.subject_key,
        county_fips: outcomePanelState.county_fips,
        action,
        occurred_on: occurredOn || null,
        reason,
        config_fingerprint: "",
      },
    })
      .then(() => {
        document.getElementById("outcome-error").textContent = "";
        document.getElementById("outcome-reason").value = "";
        return loadOutcomeHistory();
      })
      .then(() => {
        document.dispatchEvent(new Event("outcomes-changed"));
      })
      .catch((err) => {
        document.getElementById("outcome-error").textContent = String(err);
      });
  });
}

document.addEventListener("DOMContentLoaded", initOutcomePanel);
