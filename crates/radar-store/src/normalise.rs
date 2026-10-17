// Legal suffixes are not stripped. LLC, INC, LP, LIMITED PARTNERSHIP,
// CORP and every other suffix stay in the name. Merging by suffix or
// by shared word is an inference presented as normalisation, and its
// failures are invisible. Three Bielinski entities and two Home Path
// entities are visibly related in the data and are deliberately left
// as separate groups.
//
// Ampersand is a spelling variant of the token AND, not a suffix
// strip. Replacing & with " AND " before the non-alphanumeric
// collapse is the only extra step. ADR-0001's decision not to strip
// legal suffixes or merge by shared word is unchanged.
//
// Government names are a separate step. Parcel rolls append a
// department, facility, or "AND <park>" fragment to CITY OF / VILLAGE
// OF / TOWN OF / COUNTY names. That is not a second owner. The
// collapse below runs only after government language matches, and
// never on a private name that happens to contain AND.

pub fn normalise_owner_name(raw: &str) -> String {
    let upper: String = raw.chars().flat_map(|ch| ch.to_uppercase()).collect();
    let with_and = upper.replace('&', " AND ");
    let mut out = String::with_capacity(with_and.len());
    let mut prev_space = true;
    for ch in with_and.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Grouping key. Same as [`normalise_owner_name`] except that a
/// government-language name is collapsed to the core government.
pub fn owner_group_key(raw: &str) -> String {
    collapse_government_owner_name(&normalise_owner_name(raw))
}

pub fn matches_government_language(normalised: &str) -> bool {
    normalised.contains("CITY OF")
        || normalised.contains("VILLAGE OF")
        || normalised.contains("TOWN OF")
        || normalised.contains("COUNTY OF")
        || normalised.contains("SCHOOL DISTRICT")
        || normalised.starts_with("WAUKESHA COUNTY")
        || normalised.starts_with("MILWAUKEE COUNTY")
}

fn is_private_holdout(normalised: &str) -> bool {
    if normalised.contains("CHURCH") {
        return true;
    }
    if normalised.contains(" LLC") || normalised.ends_with(" LLC") {
        return true;
    }
    if normalised.contains("LAND CONSERVANCY")
        || normalised.contains("TECHNICAL COLLEGE")
        || normalised.starts_with("BANK ")
        || normalised.starts_with("ALEXIAN ")
        || normalised.starts_with("NORTH TOWN OF ")
    {
        return true;
    }
    false
}

fn is_government_unit(s: &str) -> bool {
    let s = s.strip_prefix("THE ").unwrap_or(s);
    s.starts_with("CITY OF ")
        || s.starts_with("VILLAGE OF ")
        || s.starts_with("TOWN OF ")
        || s.starts_with("COUNTY OF ")
        || s.starts_with("SCHOOL DISTRICT")
        || s.contains(" SCHOOL DISTRICT")
}

fn cut_and_if_not_second_government(name: &str) -> String {
    let Some(idx) = name.find(" AND ") else {
        return name.to_string();
    };
    let right = &name[idx + 5..];
    if is_government_unit(right) {
        return name.to_string();
    }
    name[..idx].to_string()
}

fn longest_place<'a>(rest: &str, places: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<&'a str> = None;
    for place in places {
        if rest == *place || rest.starts_with(&format!("{place} ")) {
            if best.map(|b| place.len() > b.len()).unwrap_or(true) {
                best = Some(*place);
            }
        }
    }
    best
}

const CITY_PLACES: &[&str] = &[
    "NEW BERLIN",
    "OAK CREEK",
    "WEST ALLIS",
    "SOUTH MILWAUKEE",
    "SO MILWAUKEE",
    "SAINT FRANCIS",
    "ST FRANCIS",
    "BROOKFIELD",
    "CUDAHY",
    "DELAFIELD",
    "FRANKLIN",
    "GLENDALE",
    "GREENFIELD",
    "MEQUON",
    "MILWAUKEE",
    "MILWAUKE",
    "MILWA",
    "MILW",
    "MKE",
    "MIL",
    "MUSKEGO",
    "OCONOMOWOC",
    "OCON",
    "PEWAUKEE",
    "WAUKESHA",
    "WAUWATOSA",
];

const VILLAGE_PLACES: &[&str] = &[
    "MENOMONEE FALLS",
    "MENONOMEE FALLS",
    "MENO FALLS",
    "OCONOMOWOC LAKE",
    "WHITEFISH BAY",
    "WEST MILWAUKEE",
    "HALES CORNERS",
    "NORTH PRAIRIE",
    "LAC LA BELLE",
    "BROWN DEER",
    "ELM GROVE",
    "FOX POINT",
    "BIG BEND",
    "RIVER HILLS",
    "BAYSIDE",
    "BUTLER",
    "CHENEQUA",
    "DOUSMAN",
    "EAGLE",
    "GREENDALE",
    "HARTLAND",
    "LANNON",
    "MERTON",
    "MUKWONAGO",
    "MUWWONAGO",
    "NASHOTAH",
    "PEWAUKEE",
    "SHOREWOOD",
    "SUMMIT",
    "SUSSEX",
    "VERNON",
    "WALES",
    "WAUKESHA",
];

const TOWN_PLACES: &[&str] = &[
    "BROOKFIELD",
    "DELAFIELD",
    "EAGLE",
    "GENESEE",
    "LISBON",
    "MERTON",
    "MUKWONAGO",
    "OCONOMOWOC",
    "OTTAWA",
    "PEWAUKEE",
];

fn alias_city(place: &str) -> &str {
    match place {
        "MILWAUKEE" | "MILWAUKE" | "MILWA" | "MILW" | "MKE" | "MIL" => "MILWAUKEE",
        "OCON" => "OCONOMOWOC",
        "SO MILWAUKEE" => "SOUTH MILWAUKEE",
        "SAINT FRANCIS" | "ST FRANCIS" => "ST FRANCIS",
        other => other,
    }
}

fn alias_village(place: &str) -> &str {
    match place {
        "MENO FALLS" | "MENONOMEE FALLS" => "MENOMONEE FALLS",
        "MUWWONAGO" => "MUKWONAGO",
        other => other,
    }
}

fn city_from_rest(rest: &str) -> Option<String> {
    if rest.starts_with("MILW") || rest.starts_with("MILWAUKE") {
        return Some("MILWAUKEE".to_string());
    }
    longest_place(rest, CITY_PLACES).map(alias_city).map(str::to_string)
}

/// Collapse a normalised name to the core government when, and only
/// when, it matches government language.
///
/// Logic:
/// 1. If the name is not government language, return it unchanged.
/// 2. If it is a church, a private LLC, a land conservancy, a
///    technical college, or another holdout that only *contains*
///    government words, return it unchanged.
/// 3. Strip a trailing `AND <fragment>` when that fragment is not
///    itself a government unit (park, facility, ATTN line). Two
///    governments joined by AND stay joined.
/// 4. If the name is `CITY OF` / `VILLAGE OF` / `TOWN OF` / `COUNTY OF`
///    plus a known place, drop every token after that place
///    (department, facility, truncated school name).
/// 5. `MILWAUKEE COUNTY` / `WAUKESHA COUNTY` keep those two tokens.
/// 6. School districts only lose an ATTN-style AND tail; district
///    names are not merged into the parent city.
pub fn collapse_government_owner_name(normalised: &str) -> String {
    if !matches_government_language(normalised) {
        return normalised.to_string();
    }
    if is_private_holdout(normalised) {
        return normalised.to_string();
    }

    let mut s = normalised.to_string();
    if let Some(rest) = s.strip_prefix("THE ") {
        s = rest.to_string();
    }
    if s.starts_with("GREENDALEVILLAGE OF") {
        s = "VILLAGE OF GREENDALE".to_string();
    }
    if let Some(place) = s.strip_suffix(" CITY OF") {
        if !s.starts_with("CITY OF ") && !place.is_empty() {
            s = format!("CITY OF {place}");
        }
    }
    s = cut_and_if_not_second_government(&s);

    if let Some(rest) = s.strip_prefix("CITY OF ") {
        if let Some(city) = city_from_rest(rest) {
            return format!("CITY OF {city}");
        }
    }
    if let Some(rest) = s.strip_prefix("VILLAGE OF ") {
        if let Some(place) = longest_place(rest, VILLAGE_PLACES) {
            return format!("VILLAGE OF {}", alias_village(place));
        }
    }
    if let Some(rest) = s.strip_prefix("TOWN OF ") {
        if let Some(place) = longest_place(rest, TOWN_PLACES) {
            return format!("TOWN OF {place}");
        }
    }
    if let Some(rest) = s.strip_prefix("COUNTY OF ") {
        if rest.starts_with("MILWAUKEE") {
            return "MILWAUKEE COUNTY".to_string();
        }
        if rest.starts_with("WAUKESHA") {
            return "WAUKESHA COUNTY".to_string();
        }
    }

    let instrumentality = s.starts_with("HOUSING")
        || s.starts_with("HOUS AUTH")
        || s.starts_with("CDA")
        || s.starts_with("REDEV")
        || s.starts_with("REDEVELOPMENT");
    if instrumentality {
        if let Some(idx) = s.find("CITY OF ") {
            if let Some(city) = city_from_rest(&s[idx + 8..]) {
                return format!("CITY OF {city}");
            }
        }
        if let Some(idx) = s.find("VILLAGE OF ") {
            if let Some(place) = longest_place(&s[idx + 11..], VILLAGE_PLACES) {
                return format!("VILLAGE OF {}", alias_village(place));
            }
        }
        if let Some(idx) = s.find("TOWN OF ") {
            if let Some(place) = longest_place(&s[idx + 8..], TOWN_PLACES) {
                return format!("TOWN OF {place}");
            }
        }
    }

    if s.starts_with("MILWAUKEE COUNTY") {
        return "MILWAUKEE COUNTY".to_string();
    }
    if s.starts_with("WAUKESHA COUNTY") {
        return "WAUKESHA COUNTY".to_string();
    }

    if s.contains("SCHOOL DISTRICT") {
        return cut_and_if_not_second_government(&s);
    }

    s
}

#[cfg(test)]
mod tests {
    use super::{normalise_owner_name, owner_group_key};

    #[test]
    fn homes_inc_variants_collapse() {
        let a = normalise_owner_name("BIELINSKI HOMES INC");
        let b = normalise_owner_name("BIELINSKI HOMES, INC.");
        assert_eq!(a, b);
        assert_eq!(a, "BIELINSKI HOMES INC");
    }

    #[test]
    fn properties_llc_does_not_collapse_with_homes() {
        let homes = normalise_owner_name("BIELINSKI HOMES INC");
        let props = normalise_owner_name("BIELINSKI PROPERTIES LLC");
        assert_ne!(homes, props);
    }

    #[test]
    fn home_path_suffixes_do_not_collapse() {
        let limited = normalise_owner_name("HOME PATH FINANCIAL LIMITED PARTNERSHIP");
        let lp = normalise_owner_name("HOME PATH FINANCIAL LP");
        assert_ne!(limited, lp);
    }

    #[test]
    fn payne_ampersand_and_word_collapse() {
        let a = normalise_owner_name("PAYNE & DOLAN INC");
        let b = normalise_owner_name("PAYNE AND DOLAN INC");
        assert_eq!(a, b);
        assert_eq!(a, "PAYNE AND DOLAN INC");
    }

    #[test]
    fn compressed_ampersand_does_not_glue_letters() {
        let key = normalise_owner_name("F&M SNYDER FAMILY LLC");
        assert_eq!(key, "F AND M SNYDER FAMILY LLC");
        assert_ne!(key, "FANDM SNYDER FAMILY LLC");
        assert_ne!(key, "FANDM");
    }

    #[test]
    fn muskego_and_lions_park_share_a_government_group() {
        let city = owner_group_key("CITY OF MUSKEGO");
        let park = owner_group_key("CITY OF MUSKEGO AND LIONS PARK");
        assert_eq!(city, park);
        assert_eq!(city, "CITY OF MUSKEGO");
    }

    #[test]
    fn non_government_and_is_not_stripped() {
        let a = owner_group_key("PAYNE AND DOLAN INC");
        let b = owner_group_key("PAYNE & DOLAN INC");
        assert_eq!(a, b);
        assert_eq!(a, "PAYNE AND DOLAN INC");
        assert_ne!(a, "PAYNE");
    }
}
