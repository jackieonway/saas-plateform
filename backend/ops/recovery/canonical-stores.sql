-- Fresh baselines are restored exactly, without historical AST reconstruction.
DO $recovery$
BEGIN
    IF NOT (SELECT count(*)=1 AND COALESCE(bool_and(fingerprint=public.stores_schema_fingerprint()),false)
            FROM public._stores_schema_contract WHERE singleton) THEN
        RAISE EXCEPTION 'restored current schema contract mismatch' USING ERRCODE='23514';
    END IF;
END
$recovery$;
