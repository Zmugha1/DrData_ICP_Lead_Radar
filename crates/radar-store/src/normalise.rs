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

#[cfg(test)]
mod tests {
    use super::normalise_owner_name;

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
}
