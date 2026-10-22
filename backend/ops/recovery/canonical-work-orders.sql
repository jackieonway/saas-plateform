-- Same guarded parse-tree reconstruction as canonical-stores.sql. The source
-- migrations and runtime's strict catalog verifier are deliberately unchanged.
BEGIN;
SET LOCAL search_path = pg_catalog,public,pg_temp;
DO $recovery$
DECLARE spec record; expected text; reparsed text; actual text;
BEGIN
  FOR spec IN SELECT * FROM (VALUES
    ('work_orders','work_orders_priority_check','priority VARCHAR(12)', 'CHECK (priority IN (''LOW'',''NORMAL'',''HIGH''))'),
    ('work_orders','work_orders_status_check','status VARCHAR(20)', 'CHECK (status IN (''OPEN'',''IN_PROGRESS'',''RESOLVED''))')
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
