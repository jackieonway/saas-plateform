-- PostgreSQL 17 deparse/reparse can distribute varchar[] -> text[] coercions
-- into array elements. Rebuild only the three known immutable-baseline IN
-- checks, after proving the target is either the original or its dump form.
BEGIN;
SET LOCAL search_path = pg_catalog,public,pg_temp;
DO $recovery$
DECLARE spec record; expected text; reparsed text; actual text;
BEGIN
  FOR spec IN SELECT * FROM (VALUES
    ('stores','stores_business_status_check','business_status VARCHAR(6)', 'CHECK (business_status IN (''OPEN'',''CLOSED''))'),
    ('store_audit_events','store_audit_events_resource_code_check','resource_code VARCHAR(32)', 'CHECK (resource_code IN (''store'',''store-staff'',''store-daily-report''))'),
    ('store_audit_events','store_audit_events_action_code_check','action_code VARCHAR(16)', 'CHECK (action_code IN (''create'',''update'',''assign'',''unassign''))')
  ) AS v(relation_name,constraint_name,column_ddl,check_ddl) LOOP
    EXECUTE 'CREATE TEMP TABLE recovery_original ('||spec.column_ddl||' '||spec.check_ddl||') ON COMMIT DROP';
    SELECT pg_get_constraintdef(oid,false) INTO STRICT expected FROM pg_constraint WHERE conrelid=to_regclass('pg_temp.recovery_original') AND contype='c';
    EXECUTE 'CREATE TEMP TABLE recovery_reparsed ('||spec.column_ddl||') ON COMMIT DROP';
    EXECUTE 'ALTER TABLE pg_temp.recovery_reparsed ADD CONSTRAINT recovery_reference '||expected;
    SELECT pg_get_constraintdef(oid,false) INTO STRICT reparsed FROM pg_constraint WHERE conrelid=to_regclass('pg_temp.recovery_reparsed') AND contype='c';
    SELECT pg_get_constraintdef(oid,false) INTO STRICT actual FROM pg_constraint WHERE conrelid=format('public.%I',spec.relation_name)::regclass AND conname=spec.constraint_name AND contype='c';
    IF actual IS NULL OR actual NOT IN (expected,reparsed) THEN
      RAISE EXCEPTION 'unsupported restored constraint; quarantine required' USING ERRCODE='23514';
    END IF;
    EXECUTE format('ALTER TABLE public.%I DROP CONSTRAINT %I',spec.relation_name,spec.constraint_name);
    EXECUTE format('ALTER TABLE public.%I ADD CONSTRAINT %I ',spec.relation_name,spec.constraint_name)||spec.check_ddl;
    SELECT pg_get_constraintdef(oid,false) INTO STRICT actual FROM pg_constraint WHERE conrelid=format('public.%I',spec.relation_name)::regclass AND conname=spec.constraint_name;
    IF actual<>expected THEN RAISE EXCEPTION 'canonical constraint mismatch'; END IF;
    DROP TABLE pg_temp.recovery_original,pg_temp.recovery_reparsed;
  END LOOP;
END
$recovery$;
COMMIT;
