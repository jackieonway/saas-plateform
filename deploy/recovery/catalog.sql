SET search_path = pg_catalog,public,pg_temp;
SELECT COALESCE(string_agg(item,E'\n' ORDER BY item),'') FROM (
 SELECT jsonb_build_object('kind','column','table',c.relname,'number',a.attnum,'name',a.attname,
 'type',format_type(a.atttypid,a.atttypmod),'notnull',a.attnotnull,'identity',a.attidentity,'generated',a.attgenerated,
 'dimensions',a.attndims,'storage',a.attstorage,'dropped',a.attisdropped,'local',a.attislocal,'inherited',a.attinhcount,
 'collation_schema',cn.nspname,'collation',co.collname,'default',pg_get_expr(d.adbin,d.adrelid))::text AS item
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum>0
 LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum LEFT JOIN pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_namespace cn ON cn.oid=co.collnamespace WHERE n.nspname='public'
 UNION ALL
 SELECT jsonb_build_object('kind','constraint','table',c.relname,'name',k.conname,'type',k.contype,'def',pg_get_constraintdef(k.oid,false),'validated',k.convalidated,'deferrable',k.condeferrable,'deferred',k.condeferred,'noinherit',k.connoinherit)::text
 FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'
 UNION ALL
 SELECT jsonb_build_object('kind','index','table',tablename,'name',indexname,'definition',indexdef)::text FROM pg_indexes WHERE schemaname='public'
 UNION ALL
 SELECT jsonb_build_object('kind','trigger','table',c.relname,'name',t.tgname,'enabled',t.tgenabled,'definition',pg_get_triggerdef(t.oid,false))::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND NOT t.tgisinternal
 UNION ALL
 SELECT jsonb_build_object('kind','function','name',p.proname,'definition',pg_get_functiondef(p.oid))::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.prokind IN ('f','p')
 UNION ALL
 SELECT jsonb_build_object('kind','relation','name',c.relname,'type',c.relkind,'persistence',c.relpersistence,'row_security',c.relrowsecurity,'force_row_security',c.relforcerowsecurity,'options',c.reloptions,'replica_identity',c.relreplident,'access_method',am.amname)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_am am ON am.oid=c.relam WHERE n.nspname='public' AND c.relkind NOT IN ('i','I','t')
 UNION ALL
 SELECT jsonb_build_object('kind','type','name',t.typname,'type',t.typtype)::text FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public'
) portable_catalog;
