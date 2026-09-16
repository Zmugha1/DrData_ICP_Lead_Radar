# License application PDF extraction report

Date: 15 September 2026
Working directory: `C:\Users\zumah\DrData_ICP_Lead_radar`
Status: demonstration on a hand-curated set. Not production ready.

See `docs/adr/ADR-0010-lira-pdf-vision-extraction.md` for why vision was used, why retrieval is a pasted URL list, why personal identifiers are excluded, and why this uses its own tables instead of ADR-0006.

No Cargo crate was added. The Anthropic Messages API is called with the existing `ureq` client. Page images are rasterized with `scripts/rasterize_pdf.py` using Python packages already on the machine.

## How to run a demonstration extract

1. Paste one ApplAttachServlet URL per line into `C:\Users\zumah\Desktop\radar_sources\milwaukee_lira_pdfs\urls.txt`.
2. Set `ANTHROPIC_API_KEY` in the process environment. Optional: `ANTHROPIC_MODEL` (default `claude-sonnet-4-5`).
3. From this repo: `cargo run -p radar-store --bin fetch_lira_pdfs` then `cargo run -p radar-store --bin extract_lira_pdfs`.
4. Review this file. Do not run the full application set until the handful is trusted.

The key is never written to a file and never placed in the prompt. The prompt is the page images plus the field schema.

Personal applicant identifiers are not in the schema: date of birth, driver license number, social security number, home phone, and owner or agent personal phone. `building_owner_phone` is the premises landlord contact only.

## Fetch

url rows=8. Downloaded only pasted ApplAttachServlet URLs.

## Extractions

record count=8

### 1413817.pdf

application_id=1413817
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:09:29
fields_unreadable=building_owner, building_owner_phone, security_camera_count, service_style, sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | Bellis Bistro | 1 | 1.00 | false |
| legal_entity_name | Bellis Bistro, LLC | 1 | 1.00 | false |
| business_address | 2643 S KINNICKINNIC AV Milwaukee WI 53207 | 1 | 1.00 | false |
| business_phone | (414) 210-3665 | 1 | 1.00 | false |
| contact_email | info@bellismke.com | 1 | 1.00 | false |
| email_is_free_provider | false | 1 | 1.00 | false |
| email_domain | bellismke.com | 1 | 1.00 | false |
| license_type | Alcohol Beverage License | 1 | 1.00 | false |
| service_style | null | null | 0.50 | true |
| sells_regulated_goods | true | 1 | 1.00 | false |
| sound_amplification | no | 13 | 0.95 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 11 | 0.95 | false |
| security_camera_count | null | null | 0.00 | true |
| security_camera_location | All over | 11 | 0.85 | false |
| building_owner | null | null | 0.00 | true |
| building_owner_phone | null | null | 0.00 | true |
| applicant_is_owner | unknown | null | 0.30 | false |

### 1477261.pdf

application_id=1477261
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:09:46
fields_unreadable=

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | CAFE MONARCA CHIVOLIN | 1 | 1.00 | false |
| legal_entity_name | CAFE MONARCA CHIVOLIN LLC | 1 | 1.00 | false |
| business_address | 1716 W LAYTON AVE, MILWAUKEE, WI 53221 | 1 | 1.00 | false |
| business_phone | (414) 544-1517 | 1 | 1.00 | false |
| contact_email | cafemonarcamke@gmail.com | 1 | 1.00 | false |
| email_is_free_provider | true | 1 | 1.00 | false |
| email_domain | gmail.com | 1 | 1.00 | false |
| license_type | Regular "Class B" Liquor | 1 | 1.00 | false |
| service_style | full_service | 9 | 1.00 | false |
| sells_regulated_goods | true | 1 | 1.00 | false |
| sound_amplification | yes | 9 | 1.00 | false |
| sound_amplification_desc | dj live music mariachi 3 bands | 16 | 0.90 | false |
| security_cameras | yes | 10 | 1.00 | false |
| security_camera_count | 11 | 10 | 1.00 | false |
| security_camera_location | INSIDE AND OUTSIDE OF THE BUILDING | 10 | 1.00 | false |
| building_owner | PATRICK K PRABHG | 11 | 1.00 | false |
| building_owner_phone | (414) 745-8594 | 11 | 1.00 | false |
| applicant_is_owner | no | 12 | 1.00 | false |

### 1484171.pdf

application_id=1484171
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:10:00
fields_unreadable=building_owner_phone, contact_email, email_domain, email_is_free_provider, sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | CARNITAS JIREH | 1 | 0.99 | false |
| legal_entity_name | CARNITAS JIREH LLC | 1 | 0.99 | false |
| business_address | 565 WEST LINCOLN AVE MILWAUKEE WI 53207 | 1 | 0.99 | false |
| business_phone | 4146431744 | 1 | 0.95 | false |
| contact_email | null | null | null | true |
| email_is_free_provider | null | null | null | true |
| email_domain | null | null | null | true |
| license_type | Food Dealer License - Restaurant | 2 | 0.95 | false |
| service_style | full_service | 2 | 0.95 | false |
| sells_regulated_goods | false | 3 | 0.99 | false |
| sound_amplification | no | 2 | 0.99 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 3 | 0.99 | false |
| security_camera_count | 6 | 3 | 0.95 | false |
| security_camera_location | all around building | 3 | 0.90 | false |
| building_owner | MARIA DE LEON | 4 | 0.99 | false |
| building_owner_phone | null | null | null | true |
| applicant_is_owner | yes | 4 | 0.95 | false |

### 1485342.pdf

application_id=1485342
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:10:15
fields_unreadable=sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | AtlantisValley Foods | 1 | 0.98 | false |
| legal_entity_name | AtlantisValley Foods, LLC | 1 | 0.98 | false |
| business_address | 8900 W Tower Ave, Milwaukee, WI 53224 | 1 | 0.95 | false |
| business_phone | 608 276 9800 | 1 | 0.98 | false |
| contact_email | brian.murry@atlantisvalley.com | 1 | 0.97 | false |
| email_is_free_provider | false | 1 | 1.00 | false |
| email_domain | atlantisvalley.com | 1 | 1.00 | false |
| license_type | Food Dealer | 5 | 0.99 | false |
| service_style | full_service | 2 | 0.95 | false |
| sells_regulated_goods | false | 3 | 0.95 | false |
| sound_amplification | no | 2 | 0.99 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 3 | 0.99 | false |
| security_camera_count | 3 | 3 | 0.98 | false |
| security_camera_location | Cafe/Micro Market | 3 | 0.97 | false |
| building_owner | Helwig Carbon | 4 | 0.98 | false |
| building_owner_phone | 414 354 2411 | 4 | 0.97 | false |
| applicant_is_owner | no | 4 | 0.95 | false |

### 1486456.pdf

application_id=1486456
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:10:30
fields_unreadable=sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | VEGAN SOUL | 1 | 0.95 | false |
| legal_entity_name | ZAKIYA COURTNEY | 1 | 0.95 | false |
| business_address | 4424 W. NORTH AVE, MILWAUKEE, WI 53210 | 1 | 0.95 | false |
| business_phone | 414-986-9529 | 1 | 0.90 | false |
| contact_email | VEGANSOULFUL@GMAIL.COM | 1 | 0.90 | false |
| email_is_free_provider | true | 1 | 1.00 | false |
| email_domain | gmail.com | 1 | 1.00 | false |
| license_type | Food Dealer License Application for Shared Kitchen Users | 2 | 0.98 | false |
| service_style | caterer | 2 | 0.95 | false |
| sells_regulated_goods | false | 5 | 0.95 | false |
| sound_amplification | no | 2 | 0.95 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 2 | 0.95 | false |
| security_camera_count | 2 | 2 | 0.90 | false |
| security_camera_location | Rear Exit, Front Dining | 2 | 0.85 | false |
| building_owner | Trickleberg Cafe | 3 | 0.85 | false |
| building_owner_phone | 414-488-2477 | 3 | 0.90 | false |
| applicant_is_owner | no | 3 | 0.95 | false |

### 1487039.pdf

application_id=1487039
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:10:44
fields_unreadable=sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | Pueblo Magico | 1 | 0.95 | false |
| legal_entity_name | Pueblo Magico LLC | 1 | 0.95 | false |
| business_address | 2531 W National Ave Milwaukee WI 53204 | 1 | 0.95 | false |
| business_phone | 414-376-5918 | 1 | 0.90 | false |
| contact_email | PuebloMagicoLLC@gmail.com | 1 | 0.90 | false |
| email_is_free_provider | true | 1 | 1.00 | false |
| email_domain | gmail.com | 1 | 1.00 | false |
| license_type | Food Dealer License for Shared Kitchen Users / Food Peddler | 2 | 0.95 | false |
| service_style | caterer | 2 | 0.95 | false |
| sells_regulated_goods | false | 2 | 0.90 | false |
| sound_amplification | no | 2 | 0.95 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 2 | 0.95 | false |
| security_camera_count | 3 | 2 | 0.90 | false |
| security_camera_location | 2 inside one outside | 2 | 0.85 | false |
| building_owner | Julio Equihua Equihua | 3 | 0.85 | false |
| building_owner_phone | (414)303-6548 | 3 | 0.85 | false |
| applicant_is_owner | no | 3 | 0.90 | false |

### 1487841.pdf

application_id=1487841
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:10:59
fields_unreadable=sound_amplification_desc

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | 876ix Nyamminz LLC | 1 | 0.95 | false |
| legal_entity_name | 876ix Nyamminz LLC | 1 | 0.95 | false |
| business_address | 520 E Center Street Milwaukee WI 53212 | 1 | 0.95 | false |
| business_phone | 262-714-9819 | 1 | 0.90 | false |
| contact_email | Nyamminz876ix@yahoo.com | 1 | 0.90 | false |
| email_is_free_provider | true | 1 | 1.00 | false |
| email_domain | yahoo.com | 1 | 1.00 | false |
| license_type | Food Dealer License for Shared Kitchen Users / Food Peddler | 2 | 0.95 | false |
| service_style | other | 2 | 0.90 | false |
| sells_regulated_goods | false | 2 | 0.95 | false |
| sound_amplification | no | 2 | 0.95 | false |
| sound_amplification_desc | null | null | null | true |
| security_cameras | yes | 2 | 0.95 | false |
| security_camera_count | 2 | 2 | 0.85 | false |
| security_camera_location | Window side of truck | 2 | 0.85 | false |
| building_owner | Youssouf Homara | 3 | 0.90 | false |
| building_owner_phone | 414-857-9433 | 3 | 0.90 | false |
| applicant_is_owner | no | 3 | 0.90 | false |

### 1489110.pdf

application_id=1489110
model=claude-sonnet-4-5
extracted_at=2026-09-15 23:11:15
fields_unreadable=applicant_is_owner, building_owner, building_owner_phone, security_camera_count, security_camera_location

| field | value | page | confidence | unreadable |
|---|---|---|---|---|
| trade_name | Salas Pub & Grill | 1 | 0.98 | false |
| legal_entity_name | Salas Pub & Grill LLC | 1 | 0.98 | false |
| business_address | 2179 S 16TH ST Milwaukee WI 53215 | 1 | 0.98 | false |
| business_phone | (414) 305-7480 | 1 | 0.98 | false |
| contact_email | salasmacias49@gmail.com | 1 | 0.98 | false |
| email_is_free_provider | true | 1 | 1.00 | false |
| email_domain | gmail.com | 1 | 1.00 | false |
| license_type | Alcohol Beverage License - Class "B" Beer and Regular "Class B" Liquor | 1 | 0.95 | false |
| service_style | tavern | 11 | 0.90 | false |
| sells_regulated_goods | true | 1 | 0.98 | false |
| sound_amplification | yes | 13 | 0.85 | false |
| sound_amplification_desc | If needed | 13 | 0.80 | false |
| security_cameras | yes | 11 | 0.90 | false |
| security_camera_count | null | null | 0.00 | true |
| security_camera_location | null | null | 0.00 | true |
| building_owner | null | null | 0.00 | true |
| building_owner_phone | null | null | 0.00 | true |
| applicant_is_owner | null | null | 0.00 | true |

## Per-field rates

| field | fill rate | unreadable rate | judgment count |
|---|---|---|---|
| trade_name | 1.00 | 0.00 | 0 |
| legal_entity_name | 1.00 | 0.00 | 0 |
| business_address | 1.00 | 0.00 | 0 |
| business_phone | 1.00 | 0.00 | 0 |
| contact_email | 0.88 | 0.12 | 1 |
| email_is_free_provider | 0.88 | 0.12 | 1 |
| email_domain | 0.88 | 0.12 | 1 |
| license_type | 1.00 | 0.00 | 0 |
| service_style | 0.88 | 0.12 | 2 |
| sells_regulated_goods | 1.00 | 0.00 | 0 |
| sound_amplification | 1.00 | 0.00 | 0 |
| sound_amplification_desc | 0.25 | 0.75 | 7 |
| security_cameras | 1.00 | 0.00 | 0 |
| security_camera_count | 0.75 | 0.25 | 2 |
| security_camera_location | 0.88 | 0.12 | 1 |
| building_owner | 0.75 | 0.25 | 2 |
| building_owner_phone | 0.62 | 0.38 | 3 |
| applicant_is_owner | 0.88 | 0.12 | 2 |

## Judgment calls

- 1484171.pdf contact_email: unreadable
- 1484171.pdf email_is_free_provider: unreadable
- 1484171.pdf email_domain: unreadable
- 1413817.pdf service_style: unreadable
- 1487841.pdf service_style: enum required a judgment
- 1413817.pdf sound_amplification_desc: unreadable
- 1484171.pdf sound_amplification_desc: unreadable
- 1485342.pdf sound_amplification_desc: unreadable
- 1486456.pdf sound_amplification_desc: unreadable
- 1487039.pdf sound_amplification_desc: unreadable
- 1487841.pdf sound_amplification_desc: unreadable
- 1489110.pdf sound_amplification_desc: confidence missing or below 0.85
- 1413817.pdf security_camera_count: unreadable
- 1489110.pdf security_camera_count: unreadable
- 1489110.pdf security_camera_location: unreadable
- 1413817.pdf building_owner: unreadable
- 1489110.pdf building_owner: unreadable
- 1413817.pdf building_owner_phone: unreadable
- 1484171.pdf building_owner_phone: unreadable
- 1489110.pdf building_owner_phone: unreadable
- 1413817.pdf applicant_is_owner: enum required a judgment
- 1489110.pdf applicant_is_owner: unreadable

## Hand comparison against the PDF

Extracted values are listed above. This run is a demonstration. Compare each non-null field to the scan before trusting volume.

