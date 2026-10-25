#!/usr/bin/env python3
"""Read-only U06 final audit. --check never opens a database connection.

--run is operator-gated until recovery is active and both source and final are
fenced. All observations stay in a new root-owned 0600 evidence file; stdout
contains counts and a digest only. No epoch, migration, HTTP or write API runs.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import time

SCRIPT = Path(__file__).resolve()
SPEC = importlib.util.spec_from_file_location('u06_audit_data', SCRIPT.with_name('uuid-u06-data.py'))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
BASE = Path('/var/lib/saas-uuid-u06-20261024')
SCRATCH = Path('/dev/shm/saas-uuid-u06-20261024')
REGISTRATION = BASE / 'operator-inputs/registration.json'
REGISTRATION_SHA256 = 'c1b2c7936af2b29bf5b1015adc8fd7c3b7365ac350a64f84a746a7c0f087600f'
FORMAT = 'saas-uuid-u06-final-audit/1'
EXPECTED = {'platform': {'uuid_columns': 107, 'foreign_keys': 64}, 'stores': {'uuid_columns': 23, 'foreign_keys': 2}, 'work_orders': {'uuid_columns': 17, 'foreign_keys': 1}}
OLD_DATABASES = tuple('qiankun_' + stem + '_preview_20261024' for stem in ('platform', 'stores', 'orders'))
OLD_ROLES = ('qiankun_preview_20261024', 'qiankun_notification_preview_20261024')
TASKS = ('application_release_operations', 'managed_deployment_jobs', 'zip_distribution_jobs', 'zip_distribution_nodes')


class Invalid(Exception):
    """Static public category; no raw identities or database errors."""


def require(value, code):
    if not value:
        raise Invalid(code)


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def read_private(path, limit=1024 * 1024):
    path = D.private_file(str(path), limit)
    D.private_dir(str(path.parent))
    before = path.lstat()
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as stream:
        held = os.fstat(stream.fileno())
        require(stat.S_ISREG(held.st_mode) and held.st_uid == 0 and stat.S_IMODE(held.st_mode) == 0o600 and held.st_nlink == 1
                and (held.st_dev, held.st_ino) == (before.st_dev, before.st_ino), 'PRIVATE_FILE_IDENTITY_CHANGED')
        raw = stream.read(limit + 1)
    require(len(raw) <= limit, 'PRIVATE_INPUT_TOO_LARGE')
    return raw


def write_new(path, value):
    path = D.checked_path(str(path))
    D.private_dir(str(path.parent))
    raw = D.canonical(value) + b'\n'
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb') as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return hashlib.sha256(raw).hexdigest()


def validate_registration(reg):
    require(isinstance(reg, dict) and set(reg) == {'format', 'system_identifier', 'groups', 'fenced_roles'}, 'CLOSED_REGISTRATION_REQUIRED')
    require(reg['format'] == D.REG_FORMAT and isinstance(reg['system_identifier'], str) and re.fullmatch(r'[0-9]+', reg['system_identifier'])
            and set(reg['groups']) == set(D.NAMES) and set(reg['fenced_roles']) == set(D.NAMES), 'REGISTRATION_INVALID')
    database_oids, owner_oids = set(), set()
    for group in D.NAMES:
        entries = reg['groups'][group]
        require(isinstance(entries, list) and len(entries) == 3, 'THREE_REGISTERED_DATABASES_REQUIRED')
        for i, db in enumerate(entries):
            require(isinstance(db, dict) and set(db) == {'kind', 'name', 'oid', 'owner', 'owner_oid', 'expected_schema_sha256', 'semantic_schema_sha256'}, 'CLOSED_DATABASE_REGISTRATION_REQUIRED')
            require(db['kind'] == D.KINDS[i] and db['name'] == D.NAMES[group][i] and db['owner'] == D.OWNERS[group][i], 'REGISTERED_DATABASE_NAME_MISMATCH')
            require(all(type(db[k]) is int and 0 < db[k] <= 0xffffffff for k in ('oid', 'owner_oid'))
                    and db['oid'] not in database_oids and db['owner_oid'] not in owner_oids, 'REGISTERED_IDENTITY_COLLISION')
            database_oids.add(db['oid'])
            owner_oids.add(db['owner_oid'])
            require(all(isinstance(db[k], str) and D.HEX.fullmatch(db[k]) for k in ('expected_schema_sha256', 'semantic_schema_sha256')), 'REGISTERED_SCHEMA_PIN_INVALID')
        require(isinstance(reg['fenced_roles'][group], list) and len(reg['fenced_roles'][group]) == 4
                and set(reg['fenced_roles'][group]) == set(D.RUNTIMES[group]), 'REGISTERED_RUNTIME_SET_MISMATCH')
    require(all(len({reg['groups'][g][i]['semantic_schema_sha256'] for g in D.NAMES}) == 1 for i in range(3)), 'SCHEMA_GENERATIONS_NOT_EQUIVALENT')
    return reg


def load_registration():
    raw = read_private(REGISTRATION)
    require(hashlib.sha256(raw).hexdigest() == REGISTRATION_SHA256, 'FROZEN_REGISTRATION_SHA256_MISMATCH')
    require(all(hashlib.sha256(p.read_text().encode()).hexdigest() == D.SCHEMA_QUERY_SHA256 for p in D.SCHEMA_SQL.values()), 'FROZEN_SCHEMA_QUERY_CHANGED')
    return validate_registration(D.parsed(raw))


def client_fence(clients):
    disabled = [c for c in clients if c['status'] == 'DISABLED']
    require(len(disabled) == 1 and disabled[0]['redirect_uris']
            and len([c for c in clients if c['client_type'] == 'CONFIDENTIAL']) == 12,
            'SOLE_DISABLED_EXTERNAL_CLIENT_FENCE_CHANGED')
    return {key: disabled[0][key] for key in ('client_id', 'client_type', 'status', 'redirect_uris')}


def load_epoch_bindings(reg):
    # Read existing immutable journals only. Importing or invoking the epoch
    # helper would unnecessarily couple a final audit to its mutation API.
    result = {'journals': {}}
    for generation in ('final', 'recovery'):
        directory = BASE / ('epoch-' + generation)
        prepared = D.parsed(read_private(directory / 'prepared.json'))
        receipt = D.parsed(read_private(directory / 'target-receipt.json'))
        activated = D.parsed(read_private(directory / 'activate/complete.json'))
        require(prepared['format'] == 'saas-uuid-u06-epoch-journal/1' and prepared['status'] == 'PREPARED'
                and prepared['generation'] == generation
                and prepared['script_sha256'] == D.sha(SCRIPT.with_name('prepare-uuid-u06-epoch.py')),
                'FROZEN_EPOCH_JOURNAL_INVALID')
        require(D.value_sha(receipt) == prepared['target_receipt_sha256']
                and receipt['format'] == 'saas-uuid-u06-target-receipts/1'
                and receipt['generation'] == generation and set(receipt['targets']) == set(D.KINDS),
                'EPOCH_RECEIPT_BINDING_CHANGED')
        for db in reg['groups'][generation]:
            target = receipt['targets'][db['kind']]
            require(target['database'] == db['name'] and target['database_oid'] == db['oid']
                    and target['owner'] == db['owner'] and target['owner_oid'] == db['owner_oid']
                    and target['schema_sha256'] == db['expected_schema_sha256'], 'EPOCH_REGISTERED_TARGET_CHANGED')
        require(activated['status'] == 'ACTIVE_EPOCH_ROLES_NOLOGIN' and activated['generation'] == generation
                and activated['epoch_sha256'] == prepared['epoch_sha256']
                and activated['before_platform_sha256'] == prepared['before_platform_sha256'],
                'COMPLETED_EPOCH_BINDING_CHANGED')
        roles = activated['roles']
        require(set(roles) == set(D.RUNTIMES[generation]) and len(set(roles.values())) == 4
                and all(type(oid) is int and 0 < oid <= 0xffffffff for oid in roles.values()),
                'EPOCH_RUNTIME_OID_BINDING_INVALID')
        result['journals'][generation] = {'prepared_sha256': D.value_sha(prepared),
                                          'activation_sha256': D.value_sha(activated), 'roles': roles}
        if generation == 'final':
            before = D.parsed(read_private(directory / 'before-platform.private.json'))
            require(D.value_sha(before) == prepared['before_platform_sha256'], 'HISTORICAL_PLATFORM_BEFORE_IMAGE_CHANGED')
            result['external_client'] = client_fence(before['clients'])
            result['external_client_sha256'] = D.value_sha(result['external_client'])
        else:
            enabled = D.parsed(read_private(directory / 'enable-login/complete.json'))
            require(enabled['status'] == 'RUNTIME_LOGIN_ENABLED' and enabled['generation'] == generation
                    and enabled['roles'] == roles, 'RECOVERY_LOGIN_COMPLETION_BINDING_CHANGED')
    return result


def uuid_sql():
    catalog = (D.BACKEND / 'ops/recovery/src/uuid_inventory/catalog.rs').read_text(encoding='utf-8')
    require(catalog.count('WITH RECURSIVE\n') == 1 and catalog.count('table_counts AS MATERIALIZED (') == 1, 'UUID_CATALOG_REUSE_BOUNDARY_CHANGED')
    prefix = 'WITH RECURSIVE\n' + catalog.split('WITH RECURSIVE\n', 1)[1].split('table_counts AS MATERIALIZED (', 1)[0]
    return prefix + """scans AS MATERIALIZED (
 SELECT c.*, x.payload::jsonb AS counts FROM uuid_columns c
 CROSS JOIN LATERAL XMLTABLE('/table/row' PASSING query_to_xml(format($scan$
 WITH vals AS MATERIALIZED (%s)
 SELECT jsonb_build_object('non_null',count(*) FILTER(WHERE value IS NOT NULL),
 'v7',count(*) FILTER(WHERE value IS NOT NULL AND (get_byte(uuid_send(value),6)>>4)=7),
 'rfc_variant',count(*) FILTER(WHERE value IS NOT NULL AND (get_byte(uuid_send(value),8)&192)=128),
 'invalid',count(*) FILTER(WHERE value IS NOT NULL AND ((get_byte(uuid_send(value),6)>>4)<>7 OR (get_byte(uuid_send(value),8)&192)<>128))) AS payload FROM vals
 $scan$,c.value_query),true,false,'') COLUMNS payload text PATH 'payload') x)
 SELECT COALESCE(jsonb_agg(jsonb_build_object('schema',schema_name,'table',table_name,'column',column_name,
 'array_layers',array_layers,'domains',domains,'has_default',default_expression IS NOT NULL,'counts',counts)
 ORDER BY schema_name,table_name,attnum),'[]'::jsonb) FROM scans;"""


FK_CATALOG = """SELECT COALESCE(jsonb_agg(jsonb_build_object('name',f.conname,'schema',cn.nspname,'table',c.relname,
 'parent_schema',pn.nspname,'parent_table',p.relname,'match',f.confmatchtype,'validated',f.convalidated,
 'columns',(SELECT jsonb_agg(jsonb_build_object('child',ca.attname,'parent',pa.attname) ORDER BY k.n)
 FROM unnest(f.conkey,f.confkey) WITH ORDINALITY k(child,parent,n)
 JOIN pg_attribute ca ON ca.attrelid=f.conrelid AND ca.attnum=k.child
 JOIN pg_attribute pa ON pa.attrelid=f.confrelid AND pa.attnum=k.parent)) ORDER BY cn.nspname,c.relname,f.conname),'[]'::jsonb)
 FROM pg_constraint f JOIN pg_class c ON c.oid=f.conrelid JOIN pg_namespace cn ON cn.oid=c.relnamespace
 JOIN pg_class p ON p.oid=f.confrelid JOIN pg_namespace pn ON pn.oid=p.relnamespace
 WHERE f.contype='f' AND cn.nspname<>'information_schema' AND left(cn.nspname,3)<>'pg_';"""


def qualified(schema, table):
    return D.ident(schema) + '.' + D.ident(table)


def orphan_sql(fk):
    require(fk['validated'] and fk['match'] in ('s', 'f') and fk['columns'], 'UNSUPPORTED_OR_UNVALIDATED_FOREIGN_KEY')
    require(len({c['child'] for c in fk['columns']}) == len(fk['columns']), 'FOREIGN_KEY_COLUMN_PAIRING_INVALID')
    present = ' AND '.join('c.' + D.ident(c['child']) + ' IS NOT NULL' for c in fk['columns'])
    all_null = ' AND '.join('c.' + D.ident(c['child']) + ' IS NULL' for c in fk['columns'])
    pairs = ' AND '.join('p.' + D.ident(c['parent']) + '=c.' + D.ident(c['child']) for c in fk['columns'])
    missing = '(' + present + ') AND NOT EXISTS(SELECT 1 FROM ' + qualified(fk['parent_schema'], fk['parent_table']) + ' p WHERE ' + pairs + ')'
    if fk['match'] == 'f':
        missing = '(NOT(' + all_null + ') AND NOT(' + present + ')) OR (' + missing + ')'
    return 'SELECT to_json(count(*)) FROM ONLY ' + qualified(fk['schema'], fk['table']) + ' c WHERE ' + missing + ';'


SEQUENCE_CATALOG = """SELECT COALESCE(jsonb_agg(jsonb_build_object('name',s.relname,'schema',n.nspname,'owner',pg_get_userbyid(s.relowner),
 'owner_oid',s.relowner::bigint,'start',q.seqstart::text,'increment',q.seqincrement::text,'min',q.seqmin::text,'max',q.seqmax::text,'cache',q.seqcache::text,'cycle',q.seqcycle,
 'owned_schema',tn.nspname,'owned_table',t.relname,'owned_column',a.attname,'dependency',d.deptype) ORDER BY n.nspname,s.relname),'[]'::jsonb)
 FROM pg_class s JOIN pg_namespace n ON n.oid=s.relnamespace JOIN pg_sequence q ON q.seqrelid=s.oid
 LEFT JOIN pg_depend d ON d.classid='pg_class'::regclass AND d.objid=s.oid AND d.refclassid='pg_class'::regclass AND d.deptype IN ('a','i')
 LEFT JOIN pg_class t ON t.oid=d.refobjid LEFT JOIN pg_namespace tn ON tn.oid=t.relnamespace
 LEFT JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum=d.refobjsubid
 WHERE s.relkind='S' AND n.nspname<>'information_schema' AND left(n.nspname,3)<>'pg_';"""


def validate_sequence(definition, state, owner, column_bounds=None, owner_oid=None):
    require(definition['owner'] == owner and (owner_oid is None or definition['owner_oid'] == owner_oid) and definition['schema'] == 'public', 'SEQUENCE_OWNERSHIP_INVALID')
    lo, hi, last, increment = map(int, (definition['min'], definition['max'], state['last_value'], definition['increment']))
    require(lo <= int(definition['start']) <= hi and lo <= last <= hi and increment != 0
            and int(definition['cache']) == 1 and type(state['is_called']) is bool and type(definition['cycle']) is bool, 'SEQUENCE_STATE_INVALID')
    next_value = last + increment if state['is_called'] else last
    if not lo <= next_value <= hi:
        require(definition['cycle'], 'SEQUENCE_EXHAUSTED')
        next_value = lo if increment > 0 else hi
    if definition['owned_table'] is not None:
        require(definition['owned_schema'] == 'public' and definition['owned_column'] and definition['dependency'] in ('a', 'i'), 'SEQUENCE_OWNED_COLUMN_INVALID')
        require(column_bounds is not None, 'SEQUENCE_COLUMN_BOUNDS_MISSING')
        edge = column_bounds['max' if increment > 0 else 'min']
        if edge is not None:
            require((next_value > int(edge)) if increment > 0 else (next_value < int(edge)), 'SEQUENCE_WOULD_COLLIDE_WITH_RETAINED_ROW')
    return {**definition, **state, 'column_bounds': column_bounds, 'legal': True}


def sequence_audit(session, owner, owner_oid=None):
    definitions = session.one(SEQUENCE_CATALOG)
    states = {s['name']: s for s in D.sequences(session)}
    require(set(states) == {s['name'] for s in definitions} and len(states) == len(definitions), 'SEQUENCE_COVERAGE_MISMATCH')
    reports = []
    for definition in definitions:
        bounds = None
        if definition['owned_table'] is not None:
            column = D.ident(definition['owned_column'])
            bounds = session.one("SELECT jsonb_build_object('min',min(" + column + ")::text,'max',max(" + column + ')::text) FROM ' + qualified(definition['owned_schema'], definition['owned_table']) + ';')
        reports.append(validate_sequence(definition, states[definition['name']], owner, bounds, owner_oid))
    return reports


def counts_sql(kind):
    tables = D.TABLES[kind]
    return 'SELECT jsonb_build_object(' + ','.join(D.literal(t) + ',(SELECT count(*) FROM ONLY public.' + D.ident(t) + ')' for t in tables) + ');'


def business_audit(session, kind):
    counts = session.one(counts_sql(kind))
    value = {'table_rows': counts}
    if kind == 'platform':
        value['clients'] = session.one("SELECT COALESCE(jsonb_agg(jsonb_build_object('client_id',client_id,'client_type',client_type,'status',status,'redirect_uris',redirect_uris) ORDER BY client_id),'[]'::jsonb) FROM public.oauth_clients;")
        value['disabled_client'] = client_fence(value['clients'])
        value['notification_publish_grants'] = session.one("SELECT to_json(count(*)) FROM public.membership_data_grants WHERE resource_code='notification' AND action_code='publish';")
        require(value['notification_publish_grants'] == 0, 'TEMPORARY_NOTIFICATION_PUBLISH_GRANT_REAPPEARED')
        value['task_rows'] = {name: counts[name] for name in TASKS}
        require(all(count == 0 for count in value['task_rows'].values()), 'EXTERNAL_TASK_EFFECTS_REQUIRE_RECONCILIATION')
    return value


def validate_uuid(scans, expected):
    require(len(scans) == expected, 'UUID_COLUMN_COVERAGE_MISMATCH')
    for scan in scans:
        counts = scan['counts']
        require(all(type(counts[k]) is int and counts[k] >= 0 for k in ('non_null', 'v7', 'rfc_variant', 'invalid')), 'UUID_SCAN_COUNT_INVALID')
        require(counts['invalid'] == 0 and counts['non_null'] == counts['v7'] == counts['rfc_variant'], 'NON_V7_OR_WRONG_VARIANT_UUID')
    return {'uuid_columns': len(scans), 'non_null_uuid_values': sum(s['counts']['non_null'] for s in scans),
            'v7_uuid_values': sum(s['counts']['v7'] for s in scans), 'rfc_variant_uuid_values': sum(s['counts']['rfc_variant'] for s in scans), 'invalid_uuid_values': 0}


def role_catalog_sql(databases, protected):
    names = ','.join(D.literal(n) for n in databases)
    protected_names = ','.join(D.literal(r) for r in protected)
    return """(SELECT jsonb_agg(jsonb_build_object('name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,
 'superuser',r.rolsuper,'createrole',r.rolcreaterole,'safe',NOT(r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),
 'memberships',(SELECT count(*) FROM pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid),
 'privileged_set',EXISTS(SELECT FROM pg_roles p WHERE (p.rolsuper OR p.rolcreaterole OR p.rolname IN (""" + protected_names + ") OR p.oid IN (SELECT datdba FROM pg_database WHERE datname IN (" + names + "))) AND (pg_has_role(r.oid,p.oid,'SET') OR pg_has_role(r.oid,p.oid,'USAGE')))," + """
 'admin_protected',EXISTS(SELECT FROM pg_auth_members e JOIN pg_roles p ON p.oid=e.roleid
 WHERE (p.rolname IN (""" + protected_names + ") OR p.oid IN (SELECT datdba FROM pg_database WHERE datname IN (" + names + "))) AND e.admin_option AND (pg_has_role(r.oid,e.member,'SET') OR pg_has_role(r.oid,e.member,'USAGE')))," + """
 'connect',(SELECT jsonb_object_agg(d.datname,has_database_privilege(r.oid,d.oid,'CONNECT')) FROM pg_database d WHERE d.datname IN (""" + names + "))) ORDER BY r.rolname) FROM pg_roles r)"


def deny_other_logins(roles, allowed=()):
    catalog = {r['name']: r for r in roles}
    require('postgres' in catalog and catalog['postgres']['login'] and catalog['postgres']['superuser'], 'MAINTENANCE_ROLE_MISSING')
    # CREATEDB alone grants no rights in an existing protected database. Match
    # the data engine's admission contract for unrelated roles; the four active
    # runtimes independently satisfy all five safe-role flags and zero edges.
    require(not any(r['name'] not in ('postgres', *allowed) and r['login']
                    and (r['superuser'] or r['createrole'] or r['privileged_set'] or r['admin_protected'] or any(r['connect'].values()))
                    for r in roles), 'OTHER_LOGIN_CAN_WRITE_OR_ESCALATE')


def old_v4_gate(cfg):
    names = ','.join(D.literal(n) for n in OLD_DATABASES)
    session = D.Session('postgres', cfg['work'], isolation='READ COMMITTED')
    try:
        proof = session.one("SELECT jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),'readonly',current_setting('transaction_read_only')::boolean,'local',inet_server_addr() IS NULL,'user',current_user,'session_user',session_user,'roles'," + role_catalog_sql(OLD_DATABASES, OLD_ROLES) + ",'databases',(SELECT jsonb_agg(jsonb_build_object('name',d.datname,'oid',d.oid::bigint,'owner',r.rolname,'owner_oid',r.oid::bigint,'owner_login',r.rolcanlogin,'owner_safe',NOT(r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),'readonly',EXISTS(SELECT FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND 'default_transaction_read_only=on'=ANY(s.setconfig)),'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=d.oid),'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=d.datname)) ORDER BY d.datname) FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname IN (" + names + ')));')
        require(proof['system_identifier'] == cfg['reg']['system_identifier'] and proof['readonly'] and proof['local']
                and proof['user'] == proof['session_user'] == 'postgres', 'ORIGINAL_V4_MAINTENANCE_CONTEXT_INVALID')
        roles = {r['name']: r for r in proof['roles']}
        require(set(OLD_ROLES) <= set(roles) and all(not roles[name]['login'] and roles[name]['safe'] for name in OLD_ROLES), 'ORIGINAL_V4_ROLES_NOT_FENCED')
        require(len(proof['databases'] or []) == 3 and {d['name'] for d in proof['databases']} == set(OLD_DATABASES)
                and all(d['readonly'] and d['owner'] in OLD_ROLES and not d['owner_login'] and d['owner_safe'] and d['backends'] == d['prepared'] == 0 for d in proof['databases']), 'ORIGINAL_V4_DATABASES_NOT_FENCED_IDLE')
        deny_other_logins(proof['roles'])
        session.close()
        return proof
    except Exception:
        session.abort()
        raise


def active_recovery_gate(cfg):
    # The active generation has live sessions. Validate identities/defaults and
    # owner admission through postgres; never impose an idle-source assertion.
    session = D.Session('postgres', cfg['work'], isolation='READ COMMITTED')
    names = ','.join(D.literal(d['name']) for d in cfg['reg']['groups']['recovery'])
    all_databases = (*OLD_DATABASES, *(name for group in D.NAMES.values() for name in group))
    all_names = ','.join(D.literal(n) for n in all_databases)
    protected = (*OLD_ROLES, *(role for group in D.RUNTIMES.values() for role in group), *(role for group in D.OWNERS.values() for role in group))
    try:
        proof = session.one("SELECT jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),'readonly',current_setting('transaction_read_only')::boolean,'local',inet_server_addr() IS NULL,'user',current_user,'session_user',session_user,'roles'," + role_catalog_sql(all_databases, protected) + ",'public_admission',(SELECT jsonb_object_agg(d.datname,EXISTS(SELECT FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type IN ('CONNECT','CREATE','TEMPORARY'))) FROM pg_database d WHERE d.datname IN (" + all_names + ")),'databases',(SELECT jsonb_agg(jsonb_build_object('name',d.datname,'oid',d.oid::bigint,'owner',r.rolname,'owner_oid',r.oid::bigint,'owner_login',r.rolcanlogin,'owner_superuser',r.rolsuper,'readonly',EXISTS(SELECT FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND 'default_transaction_read_only=on'=ANY(s.setconfig)),'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=d.oid),'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=d.datname)) ORDER BY d.datname) FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname IN (" + names + ')));')
        require(proof['system_identifier'] == cfg['reg']['system_identifier'] and proof['readonly'] and proof['local'] and proof['user'] == proof['session_user'] == 'postgres', 'ACTIVE_RECOVERY_MAINTENANCE_CONTEXT_INVALID')
        expected = {d['name']: d for d in cfg['reg']['groups']['recovery']}
        require(len(proof['databases'] or []) == 3, 'ACTIVE_RECOVERY_DATABASE_MISSING')
        for db in proof['databases']:
            require(all(db[k] == expected[db['name']][k] for k in ('name', 'oid', 'owner', 'owner_oid'))
                    and db['readonly'] and not db['owner_login'] and not db['owner_superuser'] and db['prepared'] == 0, 'ACTIVE_RECOVERY_IDENTITY_OR_OWNER_FENCE_INVALID')
        validate_recovery_roles(proof, cfg['epoch_bindings']['journals']['recovery']['roles'], all_databases)
        session.close()
        return proof
    except Exception:
        session.abort()
        raise


def validate_recovery_roles(proof, expected, databases):
    require(set(proof['public_admission']) == set(databases) and not any(proof['public_admission'].values()), 'PROTECTED_DATABASE_PUBLIC_ADMISSION_PRESENT')
    roles = {r['name']: r for r in proof['roles']}
    require(set(expected) == set(D.RUNTIMES['recovery']) and set(expected) <= set(roles), 'ACTIVE_RECOVERY_RUNTIME_MISSING')
    for i, name in enumerate(D.RUNTIMES['recovery']):
        role = roles[name]
        own_db = D.NAMES['recovery'][i if i < 3 else 0]
        require(role['oid'] == expected[name] and role['login'] and role['safe'] and role['memberships'] == 0, 'ACTIVE_RECOVERY_RUNTIME_IDENTITY_OR_FLAGS_INVALID')
        require(set(role['connect']) == set(databases) and all(allowed is (database == own_db) for database, allowed in role['connect'].items()), 'ACTIVE_RECOVERY_RUNTIME_CROSS_DATABASE_CONNECT')
    deny_other_logins(proof['roles'], D.RUNTIMES['recovery'])


def recovery_acl_audit(session, kind, expected_roles):
    values = ','.join('(' + D.literal(service) + ',' + D.literal(role) + ')' for service, role in zip((*D.KINDS, 'notification'), D.RUNTIMES['recovery']))
    proof = session.one("""WITH specs(service,name) AS (VALUES """ + values + """), db AS (SELECT * FROM pg_database WHERE datname=current_database())
 SELECT jsonb_agg(jsonb_build_object('service',s.service,'name',r.rolname,'oid',r.oid::bigint,
 'readonly',(SELECT split_part(v,'=',2) FROM pg_db_role_setting x CROSS JOIN LATERAL unnest(x.setconfig) v WHERE x.setdatabase=db.oid AND x.setrole=r.oid AND v LIKE 'default_transaction_read_only=%'),
 'database_ddl',has_database_privilege(r.oid,db.oid,'CREATE,TEMPORARY'),
 'database_grantable',has_database_privilege(r.oid,db.oid,'CONNECT WITH GRANT OPTION,CREATE WITH GRANT OPTION,TEMPORARY WITH GRANT OPTION'),
 'schema_usage',has_schema_privilege(r.oid,'public','USAGE'),'schema_create',has_schema_privilege(r.oid,'public','CREATE'),
 'schema_grantable',has_schema_privilege(r.oid,'public','USAGE WITH GRANT OPTION,CREATE WITH GRANT OPTION'),
 'owner_reachable',pg_has_role(r.oid,db.datdba,'SET') OR pg_has_role(r.oid,db.datdba,'USAGE'),
 'tables',(SELECT jsonb_object_agg(c.relname,jsonb_build_object('select',has_table_privilege(r.oid,c.oid,'SELECT'),
 'insert',has_table_privilege(r.oid,c.oid,'INSERT'),'update',has_table_privilege(r.oid,c.oid,'UPDATE'),'delete',has_table_privilege(r.oid,c.oid,'DELETE'),
 'dangerous',has_table_privilege(r.oid,c.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN'),
 'column_select',has_any_column_privilege(r.oid,c.oid,'SELECT'),'column_insert',has_any_column_privilege(r.oid,c.oid,'INSERT'),
 'column_update',has_any_column_privilege(r.oid,c.oid,'UPDATE'),'column_references',has_any_column_privilege(r.oid,c.oid,'REFERENCES'),
 'grantable',has_table_privilege(r.oid,c.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION,DELETE WITH GRANT OPTION,TRUNCATE WITH GRANT OPTION,TRIGGER WITH GRANT OPTION,REFERENCES WITH GRANT OPTION,MAINTAIN WITH GRANT OPTION')
 OR has_any_column_privilege(r.oid,c.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION,REFERENCES WITH GRANT OPTION')))
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p')),
 'sequences',(SELECT COALESCE(jsonb_object_agg(c.relname,jsonb_build_object('usage',has_sequence_privilege(r.oid,c.oid,'USAGE'),
 'select',has_sequence_privilege(r.oid,c.oid,'SELECT'),'update',has_sequence_privilege(r.oid,c.oid,'UPDATE'),
 'grantable',has_sequence_privilege(r.oid,c.oid,'USAGE WITH GRANT OPTION,SELECT WITH GRANT OPTION,UPDATE WITH GRANT OPTION'))),'{}'::jsonb)
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='S')) ORDER BY s.service)
 FROM specs s JOIN pg_roles r ON r.rolname=s.name CROSS JOIN db;""")
    require(len(proof or []) == 4 and {r['name'] for r in proof} == set(expected_roles), 'ACTIVE_RECOVERY_ACL_ROLE_COVERAGE_MISMATCH')
    for role in proof:
        own = role['service'] == kind
        notification = role['service'] == 'notification' and kind == 'platform'
        require(role['oid'] == expected_roles[role['name']] and not any(role[k] for k in ('database_ddl', 'database_grantable', 'schema_create', 'schema_grantable', 'owner_reachable')), 'ACTIVE_RECOVERY_DDL_OWNER_OR_GRANT_OPTION')
        if own or notification:
            require(role['schema_usage'] and role['readonly'] == ('on' if notification else 'off'), 'ACTIVE_RECOVERY_SESSION_DEFAULT_OR_SCHEMA_USAGE')
        require(set(role['tables']) == set(D.TABLES[kind]), 'ACTIVE_RECOVERY_ACL_TABLE_COVERAGE_MISMATCH')
        for table, acl in role['tables'].items():
            selected = own or (notification and table == 'notifications')
            writable = own and table not in D.METADATA[kind]
            require(acl['select'] is selected and acl['column_select'] is selected
                    and all(acl[op] is writable for op in ('insert', 'update', 'delete', 'column_insert', 'column_update'))
                    and not any(acl[k] for k in ('dangerous', 'column_references', 'grantable')), 'ACTIVE_RECOVERY_TABLE_OR_COLUMN_PRIVILEGE_CHANGED')
        for acl in role['sequences'].values():
            require(acl['usage'] is own and acl['select'] is own and not acl['update'] and not acl['grantable'], 'ACTIVE_RECOVERY_SEQUENCE_PRIVILEGE_CHANGED')
    return proof


def run_audit(cfg):
    report = {'format': FORMAT, 'status': 'RUNNING', 'started_at': now(), 'registration_sha256': REGISTRATION_SHA256,
              'data_engine_sha256': D.sha(SCRIPT.with_name('uuid-u06-data.py')), 'groups': {}, 'scope': 'LOCAL_LOGICAL_RECOVERY_POST_ACTIVATION_READ_ONLY',
              'epoch_journal_bindings': cfg['epoch_bindings']['journals'], 'external_client_sha256': cfg['epoch_bindings']['external_client_sha256']}
    cfg['partial_report'] = report
    report['original_v4_before'] = old_v4_gate(cfg)
    for group in ('source', 'final', 'recovery'):
        before = D.admission(cfg, group) if group != 'recovery' else active_recovery_gate(cfg)
        reports = []
        for db in cfg['reg']['groups'][group]:
            report['current_observation'] = {'group': group, 'kind': db['kind']}
            session = D.Session(db['name'], cfg['work'])
            try:
                observed = D.inspect(session, db, cfg)
                kind = db['kind']
                scans = session.one(uuid_sql())
                totals = validate_uuid(scans, EXPECTED[kind]['uuid_columns'])
                foreign_keys = session.one(FK_CATALOG)
                require(len(foreign_keys) == EXPECTED[kind]['foreign_keys'], 'FOREIGN_KEY_COVERAGE_MISMATCH')
                orphans = [{**fk, 'orphan_rows': session.one(orphan_sql(fk))} for fk in foreign_keys]
                require(all(f['orphan_rows'] == 0 for f in orphans), 'FOREIGN_KEY_ORPHAN_FOUND')
                item = {'kind': kind, 'inspection': observed, 'uuid_scans': scans, 'totals': {**totals, 'tables': len(observed['tables']), 'foreign_keys': len(orphans), 'orphan_rows': 0},
                        'foreign_keys': orphans, 'sequences': sequence_audit(session, db['owner'], db['owner_oid']), 'business': business_audit(session, kind)}
                if group == 'recovery':
                    item['runtime_acl'] = recovery_acl_audit(session, kind, cfg['epoch_bindings']['journals']['recovery']['roles'])
                final_observed = D.inspect(session, db, cfg)
                require(final_observed == observed, 'AUDIT_SCHEMA_OR_IDENTITY_CHANGED')
                session.close()
                reports.append(item)
            except Exception:
                session.abort()
                raise
        after = D.admission(cfg, group) if group != 'recovery' else active_recovery_gate(cfg)
        # Inactive admission catalogs are stable. Active backend counts can move.
        if group != 'recovery':
            require(before == after, 'INACTIVE_ADMISSION_CHANGED_DURING_AUDIT')
        keys = ('tables', 'uuid_columns', 'non_null_uuid_values', 'v7_uuid_values', 'rfc_variant_uuid_values', 'invalid_uuid_values', 'foreign_keys', 'orphan_rows')
        totals = {key: sum(d['totals'][key] for d in reports) for key in keys}
        require(totals['tables'] == 54 and totals['uuid_columns'] == 147 and totals['foreign_keys'] == 67, 'GROUP_AUDIT_COVERAGE_MISMATCH')
        report['groups'][group] = {'admission_before': before, 'admission_after': after, 'databases': reports, 'totals': totals}
    baseline = cfg['epoch_bindings']['external_client']
    require(all(report['groups'][g]['databases'][0]['business']['disabled_client'] == baseline for g in D.NAMES), 'EXTERNAL_CLIENT_IDENTITY_OR_FENCE_CHANGED')
    report['original_v4_after'] = old_v4_gate(cfg)
    require(report['original_v4_before'] == report['original_v4_after'], 'ORIGINAL_V4_FENCE_CHANGED')
    report['status'] = 'PASS'
    del report['current_observation']
    report['completed_at'] = now()
    report['limits'] = ['Per-database repeatable-read observations are not a cross-database atomic snapshot.', 'Active recovery may create legitimate new sessions and audit rows while this read-only audit runs.', 'Sequence state is a fresh observation outside MVCC; cache=1 and no runtime sequence UPDATE are independently checked.', 'Zero database task rows do not establish unobserved external effects or contact external owners.']
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument('--check', action='store_true')
    actions.add_argument('--run', action='store_true')
    parser.add_argument('--output', help='new root-owned 0600 JSON under the fixed data root; its parent must exist root/0700')
    args = parser.parse_args(argv)
    require(sys.platform == 'linux' and os.geteuid() == 0 and 'microsoft' in os.uname().release.lower(), 'LOCAL_WSL_ROOT_REQUIRED')
    os.umask(0o077)
    registration = load_registration()
    epoch_bindings = load_epoch_bindings(registration)
    if args.check:
        require(not args.output, 'CHECK_HAS_NO_OUTPUT_OR_DATABASE_ACCESS')
        print(json.dumps({'status': 'OFFLINE_AUDIT_CONTRACT_READY', 'groups': 3, 'databases': 9, 'tables_per_group': 54, 'uuid_columns_per_group': 147, 'foreign_keys_per_group': 67}, sort_keys=True))
        return 0
    require(args.output is not None, 'NEW_PRIVATE_EVIDENCE_OUTPUT_REQUIRED')
    output = D.checked_path(args.output)
    require(output.is_relative_to(BASE) and not output.exists() and not output.is_symlink(), 'NEW_EVIDENCE_UNDER_FIXED_ROOT_REQUIRED')
    D.private_dir(str(output.parent))
    D.private_dir(str(SCRATCH), tmpfs=True)
    work = tempfile.mkdtemp(prefix='u06-final-audit-', dir=SCRATCH)
    cfg = {'reg': registration, 'fixture': False, 'work': work, 'epoch_bindings': epoch_bindings}
    started = time.monotonic()
    try:
        result = run_audit(cfg)
        result['elapsed_seconds'] = round(time.monotonic() - started, 3)
        result['script_sha256'] = D.sha(SCRIPT)
        digest = write_new(output, result)
        print(json.dumps({'status': 'PASS', 'groups': 3, 'databases': 9, 'totals': {g: r['totals'] for g, r in result['groups'].items()}, 'evidence_sha256': digest}, sort_keys=True))
        return 0
    except Exception as error:
        write_new(output, {'format': FORMAT, 'status': 'FAILED', 'registration_sha256': REGISTRATION_SHA256, 'error': str(error) if isinstance(error, (Invalid, D.Invalid)) else 'PRIVATE_READ_ONLY_AUDIT_FAILED', 'private_work': work, 'partial': cfg.get('partial_report'), 'elapsed_seconds': round(time.monotonic() - started, 3)})
        raise


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (Invalid, D.Invalid, OSError, ValueError, KeyError, TypeError):
        print(json.dumps({'status': 'FAILED', 'error': 'PRIVATE_READ_ONLY_AUDIT_FAILED'}), file=sys.stderr)
        raise SystemExit(1)
