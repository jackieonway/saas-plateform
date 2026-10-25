//! Small offline adversarial fixtures; no preview catalog or database is used.
use super::*;

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let parent = std::env::temp_dir();
        let path = parent.join(format!("uuid-transform-adversarial-{}", Uuid::now_v7()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).unwrap();
        Self(path)
    }

    fn write(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("rows.ndjson");
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if self.0.parent() == Some(std::env::temp_dir().as_path())
            && self.0.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("uuid-transform-adversarial-")
            })
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn reviewed_registry() -> Value {
    serde_json::from_str(include_str!("../../../uuid-references-v2.json")).unwrap()
}

#[test]
fn row_count_and_callback_errors_stop_consumption_at_the_exact_boundary() {
    let fixture = FixtureDirectory::new();
    let path = fixture.write(
        b"{\"row\":{\"n\":1},\"createdAtMs\":null}\n{\"row\":{\"n\":2},\"createdAtMs\":null}\n",
    );
    let mut indices = Vec::new();
    visit_rows(&path, 2, |index, row| {
        indices.push((index, row.row["n"].as_u64().unwrap()));
        Ok(())
    })
    .unwrap();
    assert_eq!(indices, [(0, 1), (1, 2)]);
    for (expected, consumed) in [(0, 0), (1, 1), (3, 2)] {
        let mut visits = 0;
        assert!(visit_rows(&path, expected, |_, _| {
            visits += 1;
            Ok(())
        })
        .is_err());
        assert_eq!(visits, consumed);
    }
    let mut visits = 0;
    let error = visit_rows(&path, 2, |_, _| {
        visits += 1;
        Err(Error("fixture visitor rejected"))
    })
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "fixture visitor rejected");
    assert_eq!(visits, 1);
    fixture.write(b"");
    visit_rows(&path, 0, |_, _| Err(Error("empty stream visited"))).unwrap();
    assert!(visit_rows(&path, 1, |_, _| Ok(())).is_err());
}

#[test]
fn invalid_envelopes_nested_duplicates_and_noninteger_times_never_reach_the_visitor() {
    let fixture = FixtureDirectory::new();
    for raw in [
        "{\"row\":[],\"createdAtMs\":null}\n",
        "{\"row\":null,\"createdAtMs\":null}\n",
        "{\"row\":{},\"createdAtMs\":null,\"extra\":true}\n",
        "{\"row\":{},\"createdAtMs\":1.0}\n",
        "{\"row\":{},\"createdAtMs\":1e3}\n",
        "{\"row\":{},\"createdAtMs\":9223372036854775808}\n",
        "{\"row\":{},\"createdAtMs\":-9223372036854775809}\n",
        "{\"row\":{},\"createdAtMs\":\"1000\"}\n",
        "{\"row\":{\"nested\":[{\"id\":1,\"id\":2}]},\"createdAtMs\":null}\n",
        "{\"row\":{\"id\":1,\"\\u0069d\":2},\"createdAtMs\":null}\n",
        "{\"row\":{},\"createdAtMs\":null}{}\n",
        "\n",
    ] {
        let path = fixture.write(raw.as_bytes());
        let mut visits = 0;
        assert!(visit_rows(&path, 1, |_, _| {
            visits += 1;
            Ok(())
        })
        .is_err());
        assert_eq!(visits, 0);
    }
    let path = fixture.write(b"{\"row\":{\"private\":\"\xff\"},\"createdAtMs\":null}\n");
    let error = visit_rows(&path, 1, |_, _| Err(Error("invalid UTF-8 visited")))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "source row UTF-8 required");
}

#[test]
fn line_cap_accepts_exact_bytes_and_rejects_one_extra_before_parsing() {
    let fixture = FixtureDirectory::new();
    let prefix = b"{\"row\":{\"text\":\"";
    let suffix = b"\"},\"createdAtMs\":null}\n";
    let content_len = MAX_LINE as usize - prefix.len() - suffix.len();
    let mut bytes = Vec::with_capacity(MAX_LINE as usize + 1);
    bytes.extend_from_slice(prefix);
    bytes.resize(prefix.len() + content_len, b'a');
    bytes.extend_from_slice(suffix);
    assert_eq!(bytes.len() as u64, MAX_LINE);
    let path = fixture.write(&bytes);
    visit_rows(&path, 1, |_, row| {
        assert_eq!(row.row["text"].as_str().unwrap().len(), content_len);
        Ok(())
    })
    .unwrap();
    bytes.insert(prefix.len(), b'b');
    fixture.write(&bytes);
    let mut visits = 0;
    assert!(visit_rows(&path, 1, |_, _| {
        visits += 1;
        Ok(())
    })
    .is_err());
    assert_eq!(visits, 0);
    fixture.write(&bytes[..bytes.len() - 2]);
    assert!(visit_rows(&path, 1, |_, _| Ok(())).is_err());
}

#[test]
fn reviewed_authorities_and_retention_are_explicit_and_do_not_infer_row_authority() {
    let registry = reviewed_registry();
    let names = authorities(&registry).unwrap();
    for authority in [
        "platform.user",
        "platform.organization",
        "platform.tenant",
        "stores.store",
        "work_orders.ticket",
    ] {
        assert!(names.contains(authority));
    }
    assert_eq!(
        names.len(),
        registry["entities"].as_array().unwrap().len()
            + registry["nonRowAuthorities"].as_array().unwrap().len()
    );
    assert_eq!(
        policy(&registry, "platform", "public", "platform_sessions").unwrap(),
        "CREDENTIAL_EXCLUDED"
    );
    assert_eq!(
        policy(&registry, "platform", "public", "audit_logs").unwrap(),
        "HISTORY_TYPED_PROJECTION"
    );
    assert_eq!(
        policy(&registry, "work_orders", "public", "work_order_counters").unwrap(),
        "RETAIN_REMAP"
    );
    for (kind, schema, table) in [
        ("unknown", "public", "users"),
        ("platform", "private", "users"),
        ("platform", "public", "unknown"),
    ] {
        assert!(policy(&registry, kind, schema, table).is_err());
    }
    let mut duplicate = registry.clone();
    let entry = duplicate["entities"][0].clone();
    duplicate["nonRowAuthorities"]
        .as_array_mut()
        .unwrap()
        .push(entry);
    assert!(authorities(&duplicate).is_err());
    let mut missing = registry;
    missing["entities"][0]
        .as_object_mut()
        .unwrap()
        .remove("authority");
    assert!(authorities(&missing).is_err());
}

#[test]
fn bounded_files_refuse_directories_relative_paths_symlinks_and_excess_bytes() {
    let fixture = FixtureDirectory::new();
    let path = fixture.write(b"12345678");
    assert_eq!(bounded_file(&path, 8).unwrap(), b"12345678");
    assert!(bounded_file(&path, 7).is_err());
    assert!(bounded_file(&fixture.0, 8).is_err());
    assert!(safe_path(Path::new("relative-private-path")).is_err());
    assert!(safe_path(&fixture.0.join("..").join("outside")).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let linked = fixture.0.join("linked");
        symlink(&path, &linked).unwrap();
        assert!(bounded_file(&linked, 8).is_err());
        let parent_link = fixture.0.join("parent-link");
        symlink(&fixture.0, &parent_link).unwrap();
        assert!(bounded_file(&parent_link.join("rows.ndjson"), 8).is_err());
    }
}

#[test]
fn streamed_rows_keep_precise_business_values_through_typed_mapping_and_proofs() {
    let fixture = FixtureDirectory::new();
    let registry = reviewed_registry();
    let old = Uuid::from_u128(0x0011_2233_4455_4000_8000_0000_0000_0001);
    let mut map = TypedMap::new(1_792_800_000_123, authorities(&registry).unwrap()).unwrap();
    let new = map.allocate("platform.tenant", old, Some(1000)).unwrap();
    let raw = format!(
        r#"{{"row":{{"tenant_id":"{old}","next_number":9223372036854775807,"amount":12345678901234567890.123456789012345678901,"artifact_sha256":"{}"}},"createdAtMs":-9223372036854775808}}
{{"row":{{"tenant_id":"{old}","next_number":9223372036854775806,"amount":0.000000000000000000000000001}},"createdAtMs":9223372036854775807}}
"#,
        "a".repeat(64)
    );
    let path = fixture.write(raw.as_bytes());
    let mut source_times = Vec::new();
    visit_rows(&path, 2, |index, envelope| {
        source_times.push(envelope.created_at_ms.unwrap());
        let mut target = envelope.row.clone();
        rows::rewrite_row(
            "work_orders",
            "work_order_counters",
            &mut target,
            &map,
            &registry,
        )?;
        rows::verify_rewrite(
            "work_orders",
            "work_order_counters",
            &envelope.row,
            &target,
            &map,
            &registry,
        )?;
        structured::transform_row(
            "work_orders",
            "work_order_counters",
            &mut target,
            &map,
            &registry,
        )?;
        let proof = super::super::reconcile::projection_proof(
            "work_orders",
            "work_order_counters",
            &envelope.row,
            &target,
            &map,
            &registry,
        )?;
        assert_eq!(target["tenant_id"], json!(new));
        assert_eq!(
            target["next_number"].to_string(),
            if index == 0 {
                "9223372036854775807"
            } else {
                "9223372036854775806"
            }
        );
        assert_eq!(
            target["amount"].to_string(),
            envelope.row["amount"].to_string()
        );
        assert_eq!(proof.transformed_columns, 1);
        assert_eq!(proof.business_sha256.len(), 64);
        Ok(())
    })
    .unwrap();
    assert_eq!(source_times, [i64::MIN, i64::MAX]);
    assert_eq!(map.len(), 1);
}
