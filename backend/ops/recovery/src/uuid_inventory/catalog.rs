//! PostgreSQL 17 catalog capture without creating temporary objects or functions.
//!
//! Dynamic scans use only catalog-quoted SELECT statements. `query_to_xml` and
//! XMLTABLE let the same SQL run through `psql -c`; a server built with libxml is
//! required. Physical table scans use ONLY so partition roots and their children
//! are never silently counted twice. No UUID row values leave the server.

pub(super) fn query() -> &'static str {
    r#"
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET TRANSACTION READ ONLY;
SET LOCAL statement_timeout = '10s';
SET LOCAL lock_timeout = '3s';
SET LOCAL row_security = off;
SET LOCAL search_path = pg_catalog;
SET LOCAL timezone = 'UTC';

WITH RECURSIVE
user_schemas AS MATERIALIZED (
    SELECT oid, nspname
    FROM pg_namespace
    WHERE nspname <> 'information_schema'
      AND left(nspname, 3) <> 'pg_'
),
tables AS MATERIALIZED (
    SELECT c.oid, n.nspname AS schema_name, c.relname AS table_name,
           c.relkind, c.relispartition
    FROM pg_class c
    JOIN user_schemas n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p')
),
column_types AS (
    SELECT t.oid AS table_oid, t.schema_name, t.table_name, a.attnum,
           a.attname AS column_name, a.atttypid AS declared_oid,
           a.atttypid AS current_oid, ARRAY[a.atttypid] AS type_path,
           ARRAY[]::text[] AS domains, 0 AS array_layers,
           NULL::oid AS first_array_oid,
           format('SELECT %I AS value FROM ONLY %I.%I',
                  a.attname, t.schema_name, t.table_name) AS value_query
    FROM tables t
    JOIN pg_attribute a ON a.attrelid = t.oid
    WHERE a.attnum > 0 AND NOT a.attisdropped
    UNION ALL
    SELECT w.table_oid, w.schema_name, w.table_name, w.attnum,
           w.column_name, w.declared_oid, next_type.oid,
           w.type_path || next_type.oid,
           CASE WHEN current_type.typtype = 'd'
                THEN w.domains || format_type(current_type.oid, NULL)
                ELSE w.domains END,
           w.array_layers + CASE WHEN current_type.typtype = 'd' THEN 0 ELSE 1 END,
           CASE WHEN current_type.typtype <> 'd'
                THEN COALESCE(w.first_array_oid, current_type.oid)
                ELSE w.first_array_oid END,
           CASE WHEN current_type.typtype = 'd' THEN
               format('SELECT value::%s AS value FROM (%s) AS domain_values',
                      format_type(next_type.oid, NULL), w.value_query)
           ELSE
               format('SELECT element AS value FROM (%s) AS array_values '
                      'CROSS JOIN LATERAL pg_catalog.unnest(array_values.value::%s) '
                      'AS elements(element)', w.value_query,
                      format_type(current_type.oid, NULL))
           END
    FROM column_types w
    JOIN pg_type current_type ON current_type.oid = w.current_oid
    JOIN pg_type next_type ON next_type.oid = CASE
        WHEN current_type.typtype = 'd' THEN current_type.typbasetype
        WHEN current_type.typcategory = 'A' THEN current_type.typelem
        ELSE 0::oid END
    WHERE NOT next_type.oid = ANY(w.type_path)
),
uuid_columns AS MATERIALIZED (
    SELECT w.*, a.attnotnull, pg_get_expr(d.adbin, d.adrelid) AS default_expression
    FROM column_types w
    JOIN pg_attribute a ON a.attrelid = w.table_oid AND a.attnum = w.attnum
    LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
    WHERE w.current_oid = 'uuid'::regtype
),
table_counts AS MATERIALIZED (
    SELECT t.*, x.row_count
    FROM tables t
    CROSS JOIN LATERAL XMLTABLE(
        '/table/row'
        PASSING query_to_xml(format(
            'SELECT count(*) AS row_count FROM ONLY %I.%I',
            t.schema_name, t.table_name), true, false, '')
        COLUMNS row_count bigint PATH 'row_count'
    ) x
),
uuid_counts AS MATERIALIZED (
    SELECT c.*, x.payload::jsonb AS counts
    FROM uuid_columns c
    CROSS JOIN LATERAL XMLTABLE(
        '/table/row'
        PASSING query_to_xml(format($scan$
            WITH uuid_values AS MATERIALIZED (%s), buckets AS (
                SELECT get_byte(uuid_send(value), 6) >> 4 AS version,
                       CASE
                         WHEN (get_byte(uuid_send(value), 8) & 128) = 0 THEN 'NCS'
                         WHEN (get_byte(uuid_send(value), 8) & 192) = 128 THEN 'RFC'
                         WHEN (get_byte(uuid_send(value), 8) & 224) = 192 THEN 'MICROSOFT'
                         ELSE 'FUTURE'
                       END AS variant,
                       value = '00000000-0000-0000-0000-000000000000'::uuid AS nil,
                       count(*) AS count
                FROM uuid_values WHERE value IS NOT NULL
                GROUP BY 1, 2, 3
            )
            SELECT jsonb_build_object(
                'column_null_count', (%s),
                'empty_array_count', %s,
                'null_value_count', (SELECT count(*) FROM uuid_values WHERE value IS NULL),
                'non_null_value_count', COALESCE((SELECT sum(count) FROM buckets), 0),
                'nil_count', COALESCE((SELECT sum(count) FROM buckets WHERE nil), 0),
                'v7_rfc_count', COALESCE((SELECT sum(count) FROM buckets
                                        WHERE version = 7 AND variant = 'RFC'), 0),
                'versions', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                    'version', version, 'variant', variant, 'nil', nil, 'count', count
                ) ORDER BY version, variant, nil) FROM buckets), '[]'::jsonb)
            ) AS payload
        $scan$, c.value_query,
            format('SELECT count(*) FROM ONLY %I.%I WHERE %I IS NULL',
                   c.schema_name, c.table_name, c.column_name),
            CASE WHEN c.first_array_oid IS NULL THEN 'null'
                 ELSE format('(SELECT count(*) FROM ONLY %I.%I '
                             'WHERE cardinality(%I::%s) = 0)',
                             c.schema_name, c.table_name, c.column_name,
                             format_type(c.first_array_oid, NULL)) END
        ), true, false, '')
        COLUMNS payload text PATH 'payload'
    ) x
),
catalog_items AS MATERIALIZED (
    SELECT 'relation'::text AS kind, n.nspname AS schema_name,
           c.relname AS object_name, ''::text AS sub_name,
           jsonb_build_object(
               'kind', 'relation', 'schema', n.nspname, 'name', c.relname,
               'relation_kind', c.relkind, 'persistence', c.relpersistence,
               'is_partition', c.relispartition,
               'partition_key', CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END,
               'partition_bound', pg_get_expr(c.relpartbound, c.oid),
               'parents', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                   'schema', pn.nspname, 'table', pc.relname) ORDER BY i.inhseqno)
                   FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent
                   JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                   WHERE i.inhrelid = c.oid), '[]'::jsonb),
               'row_security', c.relrowsecurity, 'force_row_security', c.relforcerowsecurity,
               'replica_identity', c.relreplident, 'options', c.reloptions,
               'access_method', am.amname
           ) AS item
    FROM pg_class c JOIN user_schemas n ON n.oid = c.relnamespace
    LEFT JOIN pg_am am ON am.oid = c.relam
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
    UNION ALL
    SELECT 'column', n.nspname, c.relname, lpad(a.attnum::text, 6, '0'),
           jsonb_build_object(
               'kind', 'column', 'schema', n.nspname, 'table', c.relname,
               'position', a.attnum, 'column', a.attname,
               'type', format_type(a.atttypid, a.atttypmod),
               'not_null', a.attnotnull, 'identity', a.attidentity,
               'generated', a.attgenerated, 'dimensions', a.attndims,
               'storage', a.attstorage, 'local', a.attislocal,
               'inherited', a.attinhcount, 'collation_schema', cn.nspname,
               'collation', co.collname, 'default', pg_get_expr(d.adbin, d.adrelid)
           )
    FROM pg_class c JOIN user_schemas n ON n.oid = c.relnamespace
    JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
    LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum
    LEFT JOIN pg_collation co ON co.oid = a.attcollation
    LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
    UNION ALL
    SELECT 'constraint', n.nspname, COALESCE(c.relname, ''), k.conname,
           jsonb_build_object(
               'kind', 'constraint', 'schema', n.nspname, 'table', c.relname,
               'name', k.conname, 'type', k.contype,
               'domain', CASE WHEN k.contypid <> 0 THEN format_type(k.contypid, NULL) END,
               'definition', pg_get_constraintdef(k.oid, false),
               'columns', (SELECT jsonb_agg(a.attname ORDER BY key.position)
                   FROM unnest(k.conkey) WITH ORDINALITY AS key(attnum, position)
                   JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = key.attnum),
               'referenced_schema', rn.nspname, 'referenced_table', rc.relname,
               'referenced_columns', (SELECT jsonb_agg(a.attname ORDER BY key.position)
                   FROM unnest(k.confkey) WITH ORDINALITY AS key(attnum, position)
                   JOIN pg_attribute a ON a.attrelid = k.confrelid AND a.attnum = key.attnum),
               'validated', k.convalidated, 'deferrable', k.condeferrable,
               'deferred', k.condeferred, 'no_inherit', k.connoinherit
           )
    FROM pg_constraint k JOIN user_schemas n ON n.oid = k.connamespace
    LEFT JOIN pg_class c ON c.oid = k.conrelid
    LEFT JOIN pg_class rc ON rc.oid = k.confrelid
    LEFT JOIN pg_namespace rn ON rn.oid = rc.relnamespace
    UNION ALL
    SELECT 'index', n.nspname, c.relname, ic.relname,
           jsonb_build_object(
               'kind', 'index', 'schema', n.nspname, 'table', c.relname,
               'name', ic.relname, 'definition', pg_get_indexdef(i.indexrelid),
               'unique', i.indisunique, 'primary', i.indisprimary,
               'valid', i.indisvalid, 'ready', i.indisready,
               'replica_identity', i.indisreplident
           )
    FROM pg_index i JOIN pg_class c ON c.oid = i.indrelid
    JOIN pg_class ic ON ic.oid = i.indexrelid
    JOIN user_schemas n ON n.oid = c.relnamespace
    UNION ALL
    SELECT 'trigger', n.nspname, c.relname, t.tgname,
           jsonb_build_object(
               'kind', 'trigger', 'schema', n.nspname, 'table', c.relname,
               'name', t.tgname, 'internal', t.tgisinternal,
               'enabled', t.tgenabled, 'definition', pg_get_triggerdef(t.oid, false)
           )
    FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
    JOIN user_schemas n ON n.oid = c.relnamespace
    UNION ALL
    SELECT 'view', n.nspname, c.relname, '', jsonb_build_object(
               'kind', 'view', 'schema', n.nspname, 'name', c.relname,
               'materialized', c.relkind = 'm', 'definition', pg_get_viewdef(c.oid, false)
           )
    FROM pg_class c JOIN user_schemas n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('v', 'm')
    UNION ALL
    SELECT 'function', n.nspname, p.proname, pg_get_function_identity_arguments(p.oid),
           jsonb_build_object(
               'kind', 'function', 'schema', n.nspname, 'name', p.proname,
               'routine_kind', p.prokind,
               'arguments', pg_get_function_identity_arguments(p.oid),
               'definition', pg_get_functiondef(p.oid),
               'security_definer', p.prosecdef, 'volatility', p.provolatile,
               'parallel', p.proparallel, 'configuration', p.proconfig
           )
    FROM pg_proc p JOIN user_schemas n ON n.oid = p.pronamespace
    WHERE p.prokind IN ('f', 'p', 'w')
    UNION ALL
    SELECT 'type', n.nspname, t.typname, '', jsonb_build_object(
               'kind', 'type', 'schema', n.nspname, 'name', t.typname,
               'type_kind', t.typtype, 'category', t.typcategory,
               'base_type', CASE WHEN t.typbasetype <> 0 THEN format_type(t.typbasetype, t.typtypmod) END,
               'element_type', CASE WHEN t.typelem <> 0 THEN format_type(t.typelem, NULL) END,
               'not_null', t.typnotnull, 'default', t.typdefault,
               'enum_labels', COALESCE((SELECT jsonb_agg(e.enumlabel ORDER BY e.enumsortorder)
                   FROM pg_enum e WHERE e.enumtypid = t.oid), '[]'::jsonb)
           )
    FROM pg_type t JOIN user_schemas n ON n.oid = t.typnamespace
    UNION ALL
    SELECT 'policy', n.nspname, c.relname, p.polname, jsonb_build_object(
               'kind', 'policy', 'schema', n.nspname, 'table', c.relname,
               'name', p.polname, 'command', p.polcmd, 'permissive', p.polpermissive,
               'roles', (SELECT jsonb_agg(CASE WHEN role_oid = 0 THEN 'public'
                                             ELSE pg_get_userbyid(role_oid) END ORDER BY role_oid)
                         FROM unnest(p.polroles) AS roles(role_oid)),
               'using', pg_get_expr(p.polqual, p.polrelid),
               'with_check', pg_get_expr(p.polwithcheck, p.polrelid)
           )
    FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid
    JOIN user_schemas n ON n.oid = c.relnamespace
    UNION ALL
    SELECT 'sequence', n.nspname, c.relname, '', jsonb_build_object(
               'kind', 'sequence', 'schema', n.nspname, 'name', c.relname,
               'type', format_type(s.seqtypid, NULL), 'start', s.seqstart,
               'increment', s.seqincrement, 'min', s.seqmin, 'max', s.seqmax,
               'cache', s.seqcache, 'cycle', s.seqcycle
           )
    FROM pg_sequence s JOIN pg_class c ON c.oid = s.seqrelid
    JOIN user_schemas n ON n.oid = c.relnamespace
),
migration_rows AS MATERIALIZED (
    SELECT t.schema_name, x.payload::jsonb AS item
    FROM tables t
    CROSS JOIN LATERAL XMLTABLE(
        '/table/row'
        PASSING query_to_xml(format(
            'SELECT jsonb_build_object(''schema'', %L, ''table'', %L, '
            '''version'', version::text, ''description'', description, '
            '''checksum'', encode(checksum, ''hex''), ''success'', success) AS payload '
            'FROM ONLY %I.%I ORDER BY version',
            t.schema_name, t.table_name, t.schema_name, t.table_name), true, false, '')
        COLUMNS payload text PATH 'payload'
    ) x
    WHERE t.table_name = '_sqlx_migrations'
),
schema_material AS MATERIALIZED (
    SELECT COALESCE(jsonb_agg(item ORDER BY kind, schema_name, object_name, sub_name),
                    '[]'::jsonb) AS material
    FROM catalog_items
)
SELECT jsonb_build_object(
    'metadata', jsonb_build_object(
        'database', current_database(), 'server_address', inet_server_addr()::text,
        'server_port', inet_server_port(),
        'server_version_num', current_setting('server_version_num')::integer,
        'current_user', current_user, 'transaction_read_only', current_setting('transaction_read_only'),
        'transaction_isolation', current_setting('transaction_isolation'),
        'current_timestamp', current_timestamp,
        'table_scan_scope', 'ONLY: physical rows; partition parents exclude descendants',
        'uuid_values', 'aggregate only; arrays recursively flattened; no raw UUID values',
        'row_security', current_setting('row_security')
    ),
    'schemas', COALESCE((SELECT jsonb_agg(nspname ORDER BY nspname) FROM user_schemas), '[]'::jsonb),
    'relations', COALESCE((SELECT jsonb_agg(jsonb_build_object(
        'schema', schema_name, 'table', table_name, 'kind', relkind,
        'is_partition', relispartition, 'row_count', row_count, 'scan_scope', 'ONLY'
    ) ORDER BY schema_name, table_name) FROM table_counts), '[]'::jsonb),
    'uuid_columns', COALESCE((SELECT jsonb_agg(jsonb_build_object(
        'schema', schema_name, 'table', table_name, 'column', column_name,
        'declared_type', format_type(declared_oid, NULL), 'domains', domains,
        'array_layers', array_layers, 'not_null', attnotnull,
        'default', default_expression, 'scan_scope', 'ONLY', 'counts', counts
    ) ORDER BY schema_name, table_name, attnum) FROM uuid_counts), '[]'::jsonb),
    'defaults', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'column' AND item->>'default' IS NOT NULL), '[]'::jsonb),
    'constraints', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'constraint'), '[]'::jsonb),
    'indexes', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'index'), '[]'::jsonb),
    'triggers', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'trigger'), '[]'::jsonb),
    'views', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'view'), '[]'::jsonb),
    'functions', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'function'), '[]'::jsonb),
    'types', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'type'), '[]'::jsonb),
    'policies', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'policy'), '[]'::jsonb),
    'sequences', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, object_name, sub_name)
        FROM catalog_items WHERE kind = 'sequence'), '[]'::jsonb),
    'migrations', COALESCE((SELECT jsonb_agg(item ORDER BY schema_name, (item->>'version')::bigint)
        FROM migration_rows), '[]'::jsonb),
    'schema_material', material,
    'schema_fingerprint_md5', md5(material::text)
)
FROM schema_material;
COMMIT;
"#
}

#[cfg(test)]
mod tests {
    use super::query;

    #[test]
    fn capture_uses_one_read_only_snapshot_without_database_objects() {
        let sql = query();
        assert_eq!(sql.matches("BEGIN ").count(), 1);
        assert_eq!(sql.matches("COMMIT;").count(), 1);
        assert!(sql.contains("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;"));
        assert!(sql.contains("SET TRANSACTION READ ONLY;"));
        assert!(sql.contains("SET LOCAL row_security = off;"));
        for forbidden in [
            "CREATE ",
            "DROP ",
            "INSERT ",
            "UPDATE ",
            "DELETE ",
            "TRUNCATE ",
            "\\gexec",
        ] {
            assert!(
                !sql.contains(forbidden),
                "unexpected mutable SQL: {forbidden}"
            );
        }
    }

    #[test]
    fn capture_uses_portable_uuid_bytes_and_catalog_quoted_identifiers() {
        let sql = query();
        assert!(sql.contains("get_byte(uuid_send(value), 6) >> 4"));
        assert!(sql.contains("get_byte(uuid_send(value), 8) & 192"));
        assert!(sql.contains("FROM ONLY %I.%I"));
        assert!(sql.contains("'schema', schema_name, 'table', table_name, 'column', column_name"));
        assert!(!sql.contains("uuid_extract_version"));
        assert!(!sql.contains("uuidv7()"));
        assert!(sql.contains("'transaction_read_only', current_setting('transaction_read_only')"));
        assert!(sql.contains("'checksum'', encode(checksum, ''hex'')"));
    }
}
