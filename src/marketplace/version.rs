#![warn(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use anyhow::{Context, Result, ensure};

pub(super) fn parse(value: &str) -> Result<[u32; 3]> {
    let mut components = value.split('.');
    let mut version = [0; 3];
    for component in &mut version {
        *component = components
            .next()
            .context("Catalog pack version has fewer than 3 components")?
            .parse()?;
    }
    ensure!(
        components.next().is_none(),
        "Catalog pack version has more than 3 components"
    );
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_exactly_three_unsigned_components() {
        assert_eq!(parse("1.02.4294967295").unwrap(), [1, 2, u32::MAX]);
        for malformed in ["1.2", "1.2.3.4", "1..3", "1.2.4294967296"] {
            assert!(parse(malformed).is_err(), "accepted {malformed}");
        }
    }
}
