//! Offline recovery for private upload staging; never opens a database or deletes a release.
use std::ffi::OsString;
use std::path::PathBuf;

struct Options {
    root: PathBuf,
    apply: bool,
}

fn parse(arguments: Vec<OsString>) -> anyhow::Result<Option<Options>> {
    if arguments.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        arguments[0] == "--micro-staging",
        "Unknown application argument; use --micro-staging --root <absolute-directory> [--apply]"
    );
    let mut root = None;
    let mut apply = false;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].to_str() {
            Some("--root") if root.is_none() => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("--root requires an absolute directory"))?;
                let path = PathBuf::from(value);
                anyhow::ensure!(
                    path.is_absolute() && path.to_str().is_some(),
                    "--root requires an absolute UTF-8 directory"
                );
                root = Some(path);
            }
            Some("--apply") if !apply => apply = true,
            _ => anyhow::bail!("Unknown or repeated staging maintenance option"),
        }
        index += 1;
    }
    Ok(Some(Options {
        root: root.ok_or_else(|| anyhow::anyhow!("Staging maintenance requires --root"))?,
        apply,
    }))
}

/// Dispatch before environment loading so offline recovery cannot accidentally start the API.
pub(crate) fn run(arguments: impl IntoIterator<Item = OsString>) -> anyhow::Result<bool> {
    let Some(options) = parse(arguments.into_iter().collect())? else {
        return Ok(false);
    };
    let root = options
        .root
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid staging root"))?;
    let report = crate::release_archives::maintain_staging(root, options.apply)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn normal_startup_and_explicit_dry_run_are_distinct() {
        assert!(parse(Vec::new()).unwrap().is_none());
        let root = std::env::temp_dir().join("micro-maintenance-parser");
        let options = parse(vec![
            "--micro-staging".into(),
            "--root".into(),
            root.clone().into_os_string(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(options.root, root);
        assert!(!options.apply);
    }

    #[test]
    fn deletion_requires_an_explicit_root_and_apply_flag() {
        for invalid in [
            vec!["--micro-staging", "--apply"],
            vec!["--micro-staging", "--root", "relative"],
            vec!["--micro-staging", "--root"],
            vec!["--apply"],
            vec!["--micro-staging", "--apply", "--apply"],
        ] {
            assert!(parse(args(&invalid)).is_err());
        }
        let root = std::env::temp_dir().join("micro-maintenance-parser");
        let options = parse(vec![
            "--micro-staging".into(),
            "--root".into(),
            root.into_os_string(),
            "--apply".into(),
        ])
        .unwrap()
        .unwrap();
        assert!(options.apply);
    }

    #[test]
    fn malformed_maintenance_never_falls_through_to_server_start() {
        assert!(run(args(&["--micro-staging", "--unknown"])).is_err());
        assert!(run(args(&["--unknown-service-mode"])).is_err());
    }
}
