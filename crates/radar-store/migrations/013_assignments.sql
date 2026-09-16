-- Assignment records are insert-only.
--
-- correspondent is the filing attorney. It is stored for provenance
-- only. It is never the business and must never be treated as one.
-- patent-assignee is the operating business.
--
-- Daily PASDL files are not fetched by HTTP. A plain GET of
-- data.uspto.gov/bulkdata/datasets/pasdl/ad{YYYYMMDD}.zip returns HTTP
-- 200 with a 20666-byte Angular shell, which looks like success and is
-- not. api.uspto.gov returns 401 without an ODP API key. The loader
-- therefore reads zip files a human already downloaded through a
-- browser session.

CREATE TABLE assignment_records (
    reel_no TEXT NOT NULL,
    frame_no TEXT NOT NULL,
    recorded_date TEXT,
    last_update_date TEXT,
    purge_indicator TEXT,
    page_count INTEGER,
    conveyance_text TEXT,
    correspondent_name TEXT,
    correspondent_address TEXT,
    source_file TEXT NOT NULL,
    load_run_id INTEGER NOT NULL REFERENCES load_runs (id),
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (reel_no, frame_no)
);

CREATE TABLE assignment_assignees (
    reel_no TEXT NOT NULL,
    frame_no TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    name TEXT NOT NULL,
    address_1 TEXT,
    address_2 TEXT,
    city TEXT,
    state TEXT,
    country_name TEXT,
    postcode TEXT,
    name_type TEXT,
    PRIMARY KEY (reel_no, frame_no, sequence),
    FOREIGN KEY (reel_no, frame_no) REFERENCES assignment_records (reel_no, frame_no)
);

CREATE TABLE assignment_assignors (
    reel_no TEXT NOT NULL,
    frame_no TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    name TEXT,
    execution_date TEXT,
    PRIMARY KEY (reel_no, frame_no, sequence),
    FOREIGN KEY (reel_no, frame_no) REFERENCES assignment_records (reel_no, frame_no)
);

CREATE TABLE assignment_properties (
    reel_no TEXT NOT NULL,
    frame_no TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    country TEXT,
    doc_number TEXT,
    kind TEXT,
    invention_title TEXT,
    PRIMARY KEY (reel_no, frame_no, sequence),
    FOREIGN KEY (reel_no, frame_no) REFERENCES assignment_records (reel_no, frame_no)
);
