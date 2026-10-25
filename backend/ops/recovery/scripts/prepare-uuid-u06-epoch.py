#!/usr/bin/env python3
"""Fixed local U06 credential epochs and quarantined runtime roles.

--check never connects to PostgreSQL. --prepare only reads restored targets and
creates new private material. Activation and admission changes are separate,
explicit commands; an interrupted phase cannot be repeated or silently resumed.
No source mutation, DROP, TRUNCATE, service operation or HTTP request exists.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import sys
import uuid
from urllib.parse import quote, unquote, urlsplit

BASE = Path('/var/lib/saas-uuid-u06-20261024')
SCRATCH = Path('/dev/shm/saas-uuid-u06-20261024')
KEYS = Path('/var/lib/saas-uuid-u06-keys-20261024')
KINDS = ('platform', 'stores', 'work_orders')
SERVICES = (*KINDS, 'notification', 'examples_oauth', 'examples_chat')
STEMS = dict(zip(KINDS, ('platform', 'stores', 'orders')))
INPUT_KEYS = {
    'platform': {'DATABASE_URL', 'JWT_SECRET', 'METRICS_TOKEN'},
    'stores': {'STORES_DATABASE_URL', 'PLATFORM_CLIENT_ID', 'PLATFORM_CLIENT_SECRET', 'STORES_METRICS_TOKEN'},
    'work_orders': {'WORK_ORDERS_DATABASE_URL', 'PLATFORM_CLIENT_ID', 'PLATFORM_CLIENT_SECRET', 'WORK_ORDERS_METRICS_TOKEN'},
    'notification': {'DATABASE_URL', 'METRICS_TOKEN'},
    'examples_oauth': {'OAUTH_AUTH_CLIENT_ID', 'OAUTH_AUTH_CLIENT_SECRET', 'OAUTH_DEVICE_CLIENT_ID', 'OAUTH_CC_CLIENT_ID', 'OAUTH_CC_CLIENT_SECRET'},
    'examples_chat': set(),
}
CREDENTIALS = ('oauth_authorization_codes', 'oauth_authorization_requests', 'oauth_device_codes', 'oauth_tokens', 'platform_session_refresh_tokens', 'platform_sessions')
METADATA = {'platform': {'_sqlx_migrations'}, 'stores': {'_sqlx_migrations', '_stores_schema_contract', '_stores_schema_contract_v2', '_stores_schema_contract_v3'}, 'work_orders': {'_sqlx_migrations', 'work_order_migrations'}}
HISTORY = {'platform': ('80706cce23384465caf0e66a2a3056b4b003f6ee1ea47ea1d8095e985efbb2c3', 12), 'stores': ('0ee5122f007f1b19e75e5fa0f8974f7da3944522473619bfdfdaf91b62b3b0e7', 3), 'work_orders': ('ceb4a5cd8aa674a201bd98f4e2831b89e20b40c91b38bbca4a35eac9c39c479a', 4)}
BACKEND = Path(__file__).resolve().parents[3]
WORKSPACE = BACKEND.parents[1]
SCHEMA_SQL = {'platform': BACKEND / 'src/database/schema_material.sql', 'stores': WORKSPACE / 'saas-stores/backend/src/imported_schema.sql', 'work_orders': WORKSPACE / 'saas-work-orders/backend/src/imported_schema.sql'}
FORMAT = 'saas-uuid-u06-epoch-journal/1'
HEX = re.compile(r'[0-9a-f]{64}\Z')


class Invalid(Exception):
    """Only static categories may be emitted publicly."""


def require(value, code):
    if not value:
        raise Invalid(code)


def fields(value, names):
    require(isinstance(value, dict) and set(value) == set(names), 'CLOSED_FIELD_SET_REQUIRED')


def unique(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, 'DUPLICATE_JSON_KEY')
        value[key] = item
    return value


def parsed(raw):
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(Invalid('NONFINITE_JSON')))
    except (ValueError, UnicodeError):
        raise Invalid('INVALID_PRIVATE_JSON') from None


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def sha(value):
    return hashlib.sha256(value if isinstance(value, bytes) else canonical(value)).hexdigest()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def checked(raw):
    raw = str(raw)
    require(raw.startswith('/') and '\\' not in raw and not any(ord(c) < 32 for c in raw)
            and all(p not in ('', '.', '..') for p in raw.split('/')[1:]), 'CANONICAL_PRIVATE_PATH_REQUIRED')
    path = Path(raw)
    require(not any(p.is_symlink() for p in (path, *path.parents)), 'SYMLINK_FORBIDDEN')
    return path


def private(path, directory=False):
    path = checked(path)
    item = path.lstat()
    require(item.st_uid == os.geteuid() and stat.S_IMODE(item.st_mode) == (0o700 if directory else 0o600)
            and (stat.S_ISDIR(item.st_mode) if directory else stat.S_ISREG(item.st_mode) and item.st_nlink == 1), 'PRIVATE_METADATA_INVALID')
    return path


def read_bytes(path, limit=16 * 1024 * 1024):
    path = private(path)
    private(path.parent, True)
    before = path.lstat()
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        held = os.fstat(stream.fileno())
        require(stat.S_ISREG(held.st_mode) and held.st_uid == os.geteuid() and stat.S_IMODE(held.st_mode) == 0o600
                and held.st_nlink == 1 and (before.st_dev, before.st_ino) == (held.st_dev, held.st_ino) and held.st_size <= limit, 'PRIVATE_FILE_IDENTITY_INVALID')
        raw = stream.read(limit + 1)
        after = os.fstat(stream.fileno())
    private(path)
    require(len(raw) <= limit and (held.st_dev, held.st_ino, held.st_size, held.st_mtime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns), 'PRIVATE_FILE_CHANGED')
    return raw


def read_json(path):
    return parsed(read_bytes(path))


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_new(path, value):
    path = checked(path)
    private(path.parent, True)
    raw = value if isinstance(value, bytes) else canonical(value) + b'\n'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        os.fchmod(stream.fileno(), 0o600)
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    sync_dir(path.parent)
    require(read_bytes(path, max(len(raw), 1)) == raw, 'PRIVATE_WRITE_READBACK_FAILED')
    return sha(raw)


def new_dir(path):
    private(path.parent, True)
    checked(path).mkdir(mode=0o700)
    os.chmod(path, 0o700)
    sync_dir(path.parent)
    return private(path, True)


def names(generation):
    require(generation in ('final', 'recovery'), 'GENERATION_INVALID')
    return {kind: {'database': f'qiankun_{stem}_uuid7_{generation}_20261024',
                   'owner': f'qiankun_{stem}_uuid7_{generation}_owner_20261024',
                   'runtime': f'qiankun_{stem}_uuid7_u06_{generation}_runtime_20261024'} for kind, stem in STEMS.items()}


def runtime_roles(generation):
    return {**{k: v['runtime'] for k, v in names(generation).items()},
            'notification': f'qiankun_notification_uuid7_u06_{generation}_runtime_20261024'}


def protected_databases():
    return tuple(f'qiankun_{stem}_{suffix}_20261024' for suffix in ('preview', 'uuid7_u04', 'uuid7_final', 'uuid7_recovery') for stem in STEMS.values())


def literal(value):
    require(isinstance(value, str) and '\0' not in value, 'SQL_LITERAL_INVALID')
    return "'" + value.replace("'", "''") + "'"


def do_sql(body):
    # A fixed dollar delimiter is unsafe around restored, free-form before images.
    return 'DO ' + literal('BEGIN\n' + body + '\nEND;') + ';'


def ident(value):
    require(isinstance(value, str) and re.fullmatch(r'[a-z_][a-z0-9_]{0,62}', value), 'SQL_IDENTIFIER_INVALID')
    return '"' + value + '"'


def validate_receipt(value, generation):
    fields(value, ('format', 'generation', 'targets'))
    require(value['format'] == 'saas-uuid-u06-target-receipts/1' and value['generation'] == generation, 'TARGET_RECEIPT_GENERATION_INVALID')
    fields(value['targets'], KINDS)
    fixed = names(generation)
    markers = set()
    for kind, target in value['targets'].items():
        fields(target, ('database', 'database_oid', 'owner', 'owner_oid', 'schema_sha256', 'import_marker'))
        require(target['database'] == fixed[kind]['database'] and target['owner'] == fixed[kind]['owner'], 'FIXED_TARGET_IDENTITY_REQUIRED')
        require(all(type(target[k]) is int and 0 < target[k] <= 0xffffffff for k in ('database_oid', 'owner_oid')), 'TARGET_OID_INVALID')
        require(isinstance(target['schema_sha256'], str) and HEX.fullmatch(target['schema_sha256']), 'TARGET_SCHEMA_INVALID')
        require(isinstance(target['import_marker'], str) and re.fullmatch(r'saas-uuid-import/1/[0-9a-f-]{36}/[0-9a-f]{64}', target['import_marker']), 'TARGET_MARKER_INVALID')
        batch = target['import_marker'].split('/')[2]
        try:
            parsed_batch = uuid.UUID(batch)
        except ValueError:
            raise Invalid('TARGET_MARKER_INVALID') from None
        require(str(parsed_batch) == batch and parsed_batch.version == 7 and parsed_batch.variant == uuid.RFC_4122, 'TARGET_MARKER_INVALID')
        markers.add(target['import_marker'])
    require(len({t['database_oid'] for t in value['targets'].values()}) == 3
            and len({t['owner_oid'] for t in value['targets'].values()}) == 3 and len(markers) == 1, 'DISTINCT_TARGET_OIDS_AND_COMMON_MARKER_REQUIRED')
    return value


def read_env(path):
    result = {}
    for line in read_bytes(path).decode('utf-8').splitlines():
        if not line or line.startswith('#'):
            continue
        require('=' in line, 'GENERATED_ENV_INVALID')
        key, value = line.split('=', 1)
        require(re.fullmatch(r'[A-Z][A-Z0-9_]*', key) and key not in result, 'GENERATED_ENV_INVALID')
        result[key] = value
    return result


def validate_config(value):
    fields(value, ('format', 'generation', 'target_receipt_file', 'source_env_files'))
    require(value['format'] == 'saas-uuid-u06-epoch-input/1', 'EPOCH_INPUT_FORMAT_INVALID')
    generation = value['generation']
    names(generation)
    fields(value['source_env_files'], SERVICES)
    source_root = Path('/var/lib/saas-uuid-u05-runtime-20261024') if generation == 'final' else Path('/var/lib/saas-uuid-u06-runtime-20261024/final')
    for service, path in value['source_env_files'].items():
        require(checked(path) == source_root / 'env' / (service + '.env'), 'PREVIOUS_GENERATED_ENV_REQUIRED')
    require(checked(value['target_receipt_file']).is_relative_to(BASE), 'TARGET_RECEIPT_ROOT_INVALID')
    return value


class Runner:
    def __init__(self, directory):
        self.directory = new_dir(directory)
        self.sequence = 0

    def sql(self, database, query, write=False):
        require(database in (*protected_databases(), 'postgres'), 'FIXED_DATABASE_REQUIRED')
        self.sequence += 1
        prefix = self.directory / f'{self.sequence:03d}'
        write_new(prefix.with_suffix('.sql'), query.encode())
        opts = '-c statement_timeout=120000 -c lock_timeout=10000 -c search_path=pg_catalog -c timezone=UTC -c default_transaction_read_only=' + ('off' if write else 'on')
        argv = ['/usr/sbin/runuser', '-u', 'postgres', '--', '/usr/bin/env', '-i', 'PATH=/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL=C.UTF-8',
                'PGPASSFILE=/dev/null', 'PGCONNECT_TIMEOUT=10', 'PSQL_HISTORY=/dev/null', 'PGOPTIONS=' + opts,
                '/usr/bin/psql', '-X', '-qAt', '--no-password', '-v', 'ON_ERROR_STOP=1', '-h', '/run/postgresql', '-p', '5432', '-U', 'postgres', '-d', database, '--file=-']
        try:
            result = subprocess.run(argv, input=query.encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    timeout=180, env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C.UTF-8'}, check=False)
        except subprocess.TimeoutExpired as error:
            write_new(prefix.with_suffix('.stdout'), error.stdout or b'')
            write_new(prefix.with_suffix('.stderr'), error.stderr or b'')
            raise Invalid('PRIVATE_DATABASE_COMMAND_TIMEOUT') from None
        write_new(prefix.with_suffix('.stdout'), result.stdout)
        write_new(prefix.with_suffix('.stderr'), result.stderr)
        write_new(prefix.with_suffix('.result.json'), {'returncode': result.returncode})
        require(result.returncode == 0, 'PRIVATE_DATABASE_COMMAND_FAILED_QUARANTINE_RETAINED')
        lines = [line for line in result.stdout.splitlines() if line]
        require(len(lines) == 1, 'ONE_PRIVATE_DATABASE_RESULT_REQUIRED')
        return parsed(lines[0])


def identity_sql():
    return """SELECT jsonb_build_object('database',current_database(),'database_oid',d.oid::bigint,'owner',r.rolname,'owner_oid',r.oid::bigint,
 'owner_nologin',NOT r.rolcanlogin,'owner_safe',NOT(r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),
 'import_marker',shobj_description(d.oid,'pg_database'),'database_readonly',EXISTS(SELECT FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND 'default_transaction_read_only=on'=ANY(s.setconfig)),
 'public_connect',EXISTS(SELECT FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type IN ('CONNECT','TEMPORARY','CREATE')),
 'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid()),'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=d.datname),
 'maintenance',current_user='postgres' AND session_user='postgres' AND inet_server_addr() IS NULL AND current_setting('port')='5432',
 'tables',(SELECT jsonb_agg(c.relname ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p')))
 FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname=current_database();"""


def inspect(runner, kind, target):
    value = runner.sql(target['database'], 'BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;\n' + identity_sql() + '\nCOMMIT;')
    require(all(value[k] == target[k] for k in ('database', 'database_oid', 'owner', 'owner_oid', 'import_marker')), 'LIVE_TARGET_IDENTITY_MISMATCH')
    require(value['maintenance'] and value['owner_nologin'] and value['owner_safe'] and value['database_readonly']
            and not value['public_connect'] and value['backends'] == value['prepared'] == 0, 'LIVE_TARGET_FENCE_INVALID')
    material = runner.sql(target['database'], 'BEGIN READ ONLY;\n' + SCHEMA_SQL[kind].read_text(encoding='utf-8') + '\nCOMMIT;')
    require(sha(material) == target['schema_sha256'], 'LIVE_RAW_SCHEMA_MISMATCH')
    history = runner.sql(target['database'], "BEGIN READ ONLY; SELECT COALESCE(jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum_sha384',encode(checksum,'hex')) ORDER BY version),'[]'::jsonb) FROM public._sqlx_migrations; COMMIT;")
    require(sha(history) == HISTORY[kind][0] and len(history) == HISTORY[kind][1], 'FROZEN_MIGRATION_HISTORY_MISMATCH')
    require(set(value['tables']) >= METADATA[kind], 'MIGRATION_METADATA_REQUIRED')
    value['schema_material'] = material
    return value


def platform_state(runner, database):
    counts = ','.join(literal(t) + ',(SELECT count(*) FROM public.' + ident(t) + ')' for t in CREDENTIALS)
    value = runner.sql(database, "BEGIN READ ONLY; SELECT jsonb_build_object('clients',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.client_id),'[]'::jsonb) FROM public.oauth_clients c),'users',(SELECT COALESCE(jsonb_agg(to_jsonb(u) ORDER BY u.id),'[]'::jsonb) FROM public.users u),'applications',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.code),'[]'::jsonb) FROM public.applications a),'credentials',jsonb_build_object(" + counts + ")); COMMIT;")
    require(value['credentials'] == dict.fromkeys(CREDENTIALS, 0), 'SIX_CREDENTIAL_TABLES_MUST_BE_EMPTY')
    clients = value['clients']
    confidential = [c for c in clients if c['client_type'] == 'CONFIDENTIAL']
    require(len(confidential) == 12 and len({c['client_id'] for c in confidential}) == 12
            and all(isinstance(c['client_secret_hash'], str) and HEX.fullmatch(c['client_secret_hash']) for c in confidential), 'TWELVE_CONFIDENTIAL_CLIENTS_REQUIRED')
    require(any(c['status'] == 'DISABLED' for c in clients), 'DISABLED_EXTERNAL_CLIENT_FENCE_REQUIRED')
    require(value['users'] and all(type(u['session_version']) is int and 0 <= u['session_version'] < 9223372036854775807 for u in value['users']), 'SESSION_VERSION_BOUNDARY_INVALID')
    return value


def cluster_proof(runner, generation, expected=None, login=False):
    roles = runtime_roles(generation)
    role_values = ','.join(literal(r) for r in roles.values())
    databases = ','.join(literal(d) for d in protected_databases())
    sql = """BEGIN READ ONLY; SELECT jsonb_build_object('roles',(SELECT COALESCE(jsonb_agg(jsonb_build_object('name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,
 'safe',NOT(r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),'memberships',(SELECT count(*) FROM pg_auth_members m WHERE m.member=r.oid OR m.roleid=r.oid),
 'connect',(SELECT jsonb_object_agg(d.datname,has_database_privilege(r.oid,d.oid,'CONNECT')) FROM pg_database d WHERE d.datname IN (""" + databases + ")) ) ORDER BY r.rolname),'[]'::jsonb) FROM pg_roles r WHERE r.rolname IN (" + role_values + ")),'public_admission',(SELECT jsonb_object_agg(d.datname,EXISTS(SELECT FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type IN ('CONNECT','TEMPORARY','CREATE'))) FROM pg_database d WHERE d.datname IN (" + databases + "))); COMMIT;"
    proof = runner.sql('postgres', sql)
    require(not any(proof['public_admission'].values()), 'PROTECTED_DATABASE_PUBLIC_ADMISSION_PRESENT')
    if expected is None:
        require(proof['roles'] == [], 'FRESH_RUNTIME_ROLES_REQUIRED')
    else:
        require({r['name'] for r in proof['roles']} == set(roles.values()), 'RUNTIME_ROLE_SET_MISMATCH')
        for role in proof['roles']:
            require(role['safe'] and role['memberships'] == 0 and role['login'] is login, 'RUNTIME_ROLE_FLAGS_INVALID')
            if expected:
                require(role['oid'] == expected[role['name']], 'RUNTIME_ROLE_OID_CHANGED')
            service = next(s for s, name in roles.items() if name == role['name'])
            own_db = names(generation)['platform' if service == 'notification' else service]['database']
            require(all(allowed is (database == own_db) for database, allowed in role['connect'].items()), 'RUNTIME_CROSS_DATABASE_CONNECT')
    return proof


def rotate_expected(state, client_hashes):
    after = parsed(canonical(state))
    for client in after['clients']:
        if client['client_type'] == 'CONFIDENTIAL':
            client['client_secret_hash'] = client_hashes[client['client_id']]
    for user in after['users']:
        user['session_version'] += 1
    return after


def privilege_proof(runner, kind, generation, tables):
    roles = runtime_roles(generation)
    specs = [(kind, roles[kind])]
    if kind == 'platform':
        specs.append(('notification', roles['notification']))
    values = ','.join('(' + literal(service) + ',' + literal(role) + ')' for service, role in specs)
    query = """BEGIN READ ONLY; WITH specs(service,name) AS (VALUES """ + values + """), db AS (SELECT * FROM pg_database WHERE datname=current_database())
 SELECT jsonb_agg(jsonb_build_object('service',s.service,'name',r.rolname,'oid',r.oid::bigint,
 'readonly',(SELECT split_part(v,'=',2) FROM pg_db_role_setting x CROSS JOIN LATERAL unnest(x.setconfig) v WHERE x.setdatabase=db.oid AND x.setrole=r.oid AND v LIKE 'default_transaction_read_only=%'),
 'database_ddl',has_database_privilege(r.oid,db.oid,'CREATE,TEMPORARY'),
 'schema_usage',has_schema_privilege(r.oid,'public','USAGE'),'schema_create',has_schema_privilege(r.oid,'public','CREATE'),
 'owner_reachable',pg_has_role(r.oid,db.datdba,'SET') OR pg_has_role(r.oid,db.datdba,'USAGE'),
 'tables',(SELECT jsonb_object_agg(c.relname,jsonb_build_object('select',has_table_privilege(r.oid,c.oid,'SELECT'),
 'insert',has_table_privilege(r.oid,c.oid,'INSERT'),'update',has_table_privilege(r.oid,c.oid,'UPDATE'),'delete',has_table_privilege(r.oid,c.oid,'DELETE'),
 'dangerous',has_table_privilege(r.oid,c.oid,'TRUNCATE,TRIGGER,REFERENCES'))) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p')),
 'sequences',(SELECT COALESCE(jsonb_object_agg(c.relname,has_sequence_privilege(r.oid,c.oid,'USAGE')),'{}'::jsonb) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='S')) ORDER BY s.service)
 FROM specs s JOIN pg_roles r ON r.rolname=s.name CROSS JOIN db; COMMIT;"""
    proof = runner.sql(names(generation)[kind]['database'], query)
    require(len(proof) == len(specs), 'RUNTIME_PRIVILEGE_PROOF_INCOMPLETE')
    for role in proof:
        notification = role['service'] == 'notification'
        require(role['schema_usage'] and not role['schema_create'] and not role['database_ddl'] and not role['owner_reachable'], 'RUNTIME_DDL_OR_OWNER_PRIVILEGE')
        require(role['readonly'] == ('on' if notification else 'off'), 'PER_DATABASE_READONLY_SETTING_INVALID')
        require(set(role['tables']) == set(tables), 'RUNTIME_TABLE_PROOF_INCOMPLETE')
        for table, acl in role['tables'].items():
            require(acl['select'] is (table == 'notifications' if notification else True), 'RUNTIME_SELECT_BOUNDARY_INVALID')
            writable = not notification and table not in METADATA[kind]
            require(all(acl[op] is writable for op in ('insert', 'update', 'delete')) and not acl['dangerous'], 'RUNTIME_DML_BOUNDARY_INVALID')
        require(all(allowed is (not notification) for allowed in role['sequences'].values()), 'RUNTIME_SEQUENCE_BOUNDARY_INVALID')
    return proof


def generate_material(state, source, generation):
    confidential = [c for c in state['clients'] if c['client_type'] == 'CONFIDENTIAL']
    used = {value for env in source.values() for value in env.values() if value}
    for service, db_key in (('platform', 'DATABASE_URL'), ('stores', 'STORES_DATABASE_URL'), ('work_orders', 'WORK_ORDERS_DATABASE_URL'), ('notification', 'DATABASE_URL')):
        password = urlsplit(source[service][db_key]).password
        if password:
            used.add(unquote(password))
    def fresh():
        value = secrets.token_urlsafe(48)
        while value in used or sha(value.encode()) in {c['client_secret_hash'] for c in confidential}:
            value = secrets.token_urlsafe(48)
        used.add(value)
        return value
    client_secrets = {c['client_id']: fresh() for c in confidential}
    hashes = {client: sha(secret.encode()) for client, secret in client_secrets.items()}
    jwt = fresh()
    passwords = {service: fresh() for service in runtime_roles(generation)}
    env = {service: {key: source[service][key] for key in INPUT_KEYS[service]} for service in SERVICES}
    fixed = names(generation)
    roles = runtime_roles(generation)
    for service, db_key in (('platform', 'DATABASE_URL'), ('stores', 'STORES_DATABASE_URL'), ('work_orders', 'WORK_ORDERS_DATABASE_URL'), ('notification', 'DATABASE_URL')):
        kind = 'platform' if service == 'notification' else service
        env[service][db_key] = f'postgresql://{roles[service]}:{quote(passwords[service], safe="")}@127.0.0.1:5432/{fixed[kind]["database"]}'
    env['platform']['JWT_SECRET'] = jwt
    for service, key in (('platform', 'METRICS_TOKEN'), ('stores', 'STORES_METRICS_TOKEN'), ('work_orders', 'WORK_ORDERS_METRICS_TOKEN'), ('notification', 'METRICS_TOKEN')):
        env[service][key] = fresh()
    for service, id_key, secret_key in (('stores', 'PLATFORM_CLIENT_ID', 'PLATFORM_CLIENT_SECRET'), ('work_orders', 'PLATFORM_CLIENT_ID', 'PLATFORM_CLIENT_SECRET'), ('examples_oauth', 'OAUTH_AUTH_CLIENT_ID', 'OAUTH_AUTH_CLIENT_SECRET'), ('examples_oauth', 'OAUTH_CC_CLIENT_ID', 'OAUTH_CC_CLIENT_SECRET')):
        client = env[service][id_key]
        require(client in client_secrets, 'SOURCE_ENV_CONFIDENTIAL_CLIENT_NOT_RESTORED')
        prior = next(c for c in confidential if c['client_id'] == client)
        require(prior['status'] == 'ENABLED' and sha(source[service][secret_key].encode()) == prior['client_secret_hash'], 'PREVIOUS_ENV_CLIENT_SECRET_HASH_MISMATCH')
        if service in ('stores', 'work_orders'):
            code = 'work-orders' if service == 'work_orders' else service
            apps = [a for a in state['applications'] if a['code'] == code]
            require(len(apps) == 1 and prior.get('application_id') == apps[0]['id'], 'RESTORED_APPLICATION_CLIENT_BINDING_MISMATCH')
        env[service][secret_key] = client_secrets[client]
    require(any(c['client_id'] == env['examples_oauth']['OAUTH_DEVICE_CLIENT_ID'] and c['client_type'] == 'PUBLIC' and c['status'] == 'ENABLED' for c in state['clients']), 'SOURCE_ENV_DEVICE_CLIENT_NOT_RESTORED')
    epoch = {'format': 'saas-uuid-u06-credential-epoch/1', 'generation': generation, 'jwt_secret_sha256': sha(jwt.encode()),
             'previous_jwt_secret_sha256': sha(source['platform']['JWT_SECRET'].encode()),
             'confidential_clients': [{'client_id': c['client_id'], 'previous_secret_hash': c['client_secret_hash'], 'new_secret_hash': hashes[c['client_id']]} for c in confidential], 'secrets_fresh': True}
    return {'env': env, 'passwords': passwords, 'client_secrets': client_secrets, 'hashes': hashes, 'epoch': epoch}


def create_roles_sql(generation, passwords):
    return '\n'.join('CREATE ROLE ' + ident(role) + ' NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD ' + literal(passwords[service]) + ';' for service, role in runtime_roles(generation).items())


def grants_sql(kind, generation, tables):
    database = names(generation)[kind]['database']
    role = runtime_roles(generation)[kind]
    lines = ['REVOKE ALL ON DATABASE ' + ident(database) + ' FROM PUBLIC;', 'REVOKE CREATE ON SCHEMA public FROM PUBLIC;',
             'GRANT CONNECT ON DATABASE ' + ident(database) + ' TO ' + ident(role) + ';',
             'GRANT USAGE ON SCHEMA public TO ' + ident(role) + ';',
             'GRANT SELECT ON ALL TABLES IN SCHEMA public TO ' + ident(role) + ';',
             'GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO ' + ident(role) + ';',
             'ALTER ROLE ' + ident(role) + ' IN DATABASE ' + ident(database) + ' SET default_transaction_read_only=off;']
    business = sorted(set(tables) - METADATA[kind])
    require(business, 'BUSINESS_TABLE_SET_EMPTY')
    lines.append('GRANT INSERT,UPDATE,DELETE ON ' + ','.join('public.' + ident(t) for t in business) + ' TO ' + ident(role) + ';')
    if kind == 'platform':
        reader = ident(runtime_roles(generation)['notification'])
        lines += ['GRANT CONNECT ON DATABASE ' + ident(database) + ' TO ' + reader + ';', 'GRANT USAGE ON SCHEMA public TO ' + reader + ';',
                  'GRANT SELECT ON public.notifications TO ' + reader + ';', 'ALTER ROLE ' + reader + ' IN DATABASE ' + ident(database) + ' SET default_transaction_read_only=on;']
    return '\n'.join(lines)


def mutation_guard(target):
    return do_sql("IF current_user<>'postgres' OR session_user<>'postgres' OR inet_server_addr() IS NOT NULL OR current_database()<>" + literal(target['database']) + " OR NOT EXISTS(SELECT FROM pg_database WHERE oid=" + str(target['database_oid']) + " AND datname=current_database() AND datdba=" + str(target['owner_oid']) + " AND shobj_description(oid,'pg_database')=" + literal(target['import_marker']) + ") THEN RAISE EXCEPTION 'U06 target identity changed'; END IF;")


def schema_guard(kind, material):
    query = SCHEMA_SQL[kind].read_text(encoding='utf-8').strip().removesuffix(';')
    return do_sql('IF (' + query + ')::jsonb <> ' + literal(canonical(material).decode()) + "::jsonb THEN RAISE EXCEPTION 'U06 raw schema changed'; END IF;")


def rotate_sql(before, hashes):
    expected = literal(canonical(before['clients']).decode())
    expected_users = literal(canonical(before['users']).decode())
    zeros = ' OR '.join('EXISTS(SELECT FROM public.' + ident(t) + ')' for t in CREDENTIALS)
    updates = '\n'.join('UPDATE public.oauth_clients SET client_secret_hash=' + literal(hashes[c['client_id']]) + ' WHERE client_id=' + literal(c['client_id']) + ' AND client_type=\'CONFIDENTIAL\';' for c in before['clients'] if c['client_type'] == 'CONFIDENTIAL')
    guard = do_sql('IF ' + zeros + " OR (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.client_id),'[]'::jsonb) FROM public.oauth_clients c)<>" + expected + "::jsonb OR (SELECT COALESCE(jsonb_agg(to_jsonb(u) ORDER BY u.id),'[]'::jsonb) FROM public.users u)<>" + expected_users + "::jsonb THEN RAISE EXCEPTION 'U06 credential before-image mismatch'; END IF;")
    after = rotate_expected(before, hashes)
    check = do_sql("IF (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.client_id),'[]'::jsonb) FROM public.oauth_clients c)<>" + literal(canonical(after['clients']).decode()) + "::jsonb OR (SELECT COALESCE(jsonb_agg(to_jsonb(u) ORDER BY u.id),'[]'::jsonb) FROM public.users u)<>" + literal(canonical(after['users']).decode()) + "::jsonb THEN RAISE EXCEPTION 'U06 credential after-image mismatch'; END IF;")
    return 'LOCK TABLE public.oauth_clients,public.users IN SHARE ROW EXCLUSIVE MODE;\n' + guard + '\n' + updates + '\nUPDATE public.users SET session_version=session_version+1;\n' + check


def phase(directory, name, intent):
    location = new_dir(directory / name)
    write_new(location / 'intent.json', {'format': FORMAT, 'action': name, 'started_at': now(), **intent})
    return location


def load_journal(directory):
    directory = checked(directory)
    require(directory.parent == BASE and directory.name in ('epoch-final', 'epoch-recovery'), 'FIXED_EPOCH_JOURNAL_REQUIRED')
    private(directory, True)
    prepared = read_json(directory / 'prepared.json')
    fields(prepared, ('format', 'generation', 'status', 'created_at', 'script_sha256', 'target_receipt_sha256', 'source_env_sha256', 'before_platform_sha256', 'epoch_sha256', 'service_env_sha256', 'client_secrets_sha256', 'role_passwords_sha256'))
    require(prepared['format'] == FORMAT and prepared['status'] == 'PREPARED' and directory.name == 'epoch-' + prepared['generation'], 'PREPARED_JOURNAL_INVALID')
    require(prepared['script_sha256'] == sha(Path(__file__).read_bytes()), 'EPOCH_HELPER_CHANGED_AFTER_PREPARATION')
    receipt = validate_receipt(read_json(directory / 'target-receipt.json'), prepared['generation'])
    require(sha(receipt) == prepared['target_receipt_sha256'], 'PREPARED_TARGET_RECEIPT_CHANGED')
    return prepared, receipt


def prepare(config, runner_factory=Runner):
    generation = config['generation']
    receipt = validate_receipt(read_json(config['target_receipt_file']), generation)
    source = {service: read_env(path) for service, path in config['source_env_files'].items()}
    require(all(INPUT_KEYS[s] <= set(source[s]) for s in SERVICES), 'PREVIOUS_ENV_FIELDS_MISSING')
    private(BASE, True)
    private(SCRATCH, True)
    private(KEYS, True)
    mount = subprocess.run(['/usr/bin/findmnt', '-n', '-o', 'FSTYPE', '--target', str(SCRATCH)], capture_output=True, text=True, check=False,
                           env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C.UTF-8'})
    require(mount.returncode == 0 and mount.stdout.strip() == 'tmpfs', 'PRIVATE_SCRATCH_MUST_BE_TMPFS')
    directory = BASE / ('epoch-' + generation)
    require(not directory.exists() and not directory.is_symlink(), 'FRESH_EPOCH_JOURNAL_REQUIRED')
    # Read-only preflight remains in scratch if data or source inputs are not ready.
    preflight = new_dir(SCRATCH / ('epoch-preflight-' + generation + '-' + secrets.token_hex(8)))
    runner = runner_factory(preflight / 'commands')
    observed = {kind: inspect(runner, kind, receipt['targets'][kind]) for kind in KINDS}
    cluster_proof(runner, generation)
    state = platform_state(runner, receipt['targets']['platform']['database'])
    material = generate_material(state, source, generation)
    new_dir(directory)
    write_new(directory / 'target-receipt.json', receipt)
    write_new(directory / 'input.json', config)
    write_new(directory / 'before-platform.private.json', state)
    write_new(directory / 'before-targets.private.json', observed)
    write_new(directory / 'role-passwords.private.json', material['passwords'])
    write_new(directory / 'client-secrets.private.json', {'format': 'saas-uuid-u06-client-secrets/1', 'generation': generation, 'clients': material['client_secrets']})
    epoch_sha = write_new(directory / 'credential-epoch.json', material['epoch'])
    for service in SERVICES:
        fields(material['env'][service], INPUT_KEYS[service])
        write_new(directory / (service + '.json'), material['env'][service])
    prepared = {'format': FORMAT, 'generation': generation, 'status': 'PREPARED', 'created_at': now(),
                'script_sha256': sha(Path(__file__).read_bytes()), 'target_receipt_sha256': sha(receipt),
                'source_env_sha256': {s: sha(read_bytes(p)) for s, p in config['source_env_files'].items()},
                'before_platform_sha256': sha(state), 'epoch_sha256': epoch_sha,
                'service_env_sha256': {s: sha(read_bytes(directory / (s + '.json'))) for s in SERVICES},
                'client_secrets_sha256': sha(read_bytes(directory / 'client-secrets.private.json')),
                'role_passwords_sha256': sha(read_bytes(directory / 'role-passwords.private.json'))}
    write_new(directory / 'prepared.json', prepared)
    return {'status': 'PREPARED_NO_DATABASE_MUTATION', 'generation': generation, 'confidential_clients': 12, 'runtime_roles': 4, 'epoch_sha256': epoch_sha}


def activate(directory, runner_factory=Runner):
    prepared, receipt = load_journal(directory)
    generation = prepared['generation']
    before = read_json(directory / 'before-platform.private.json')
    require(sha(before) == prepared['before_platform_sha256'], 'PLATFORM_BEFORE_IMAGE_CHANGED')
    epoch_raw = read_bytes(directory / 'credential-epoch.json')
    require(sha(epoch_raw) == prepared['epoch_sha256'], 'EPOCH_DTO_CHANGED')
    epoch = parsed(epoch_raw)
    hashes = {c['client_id']: c['new_secret_hash'] for c in epoch['confidential_clients']}
    passwords_raw = read_bytes(directory / 'role-passwords.private.json')
    require(sha(passwords_raw) == prepared['role_passwords_sha256'], 'ROLE_PASSWORDS_CHANGED')
    passwords = parsed(passwords_raw)
    require(sha(read_bytes(directory / 'client-secrets.private.json')) == prepared['client_secrets_sha256'], 'CLIENT_SECRET_INPUT_CHANGED')
    for service, expected_hash in prepared['service_env_sha256'].items():
        require(sha(read_bytes(directory / (service + '.json'))) == expected_hash, 'SERVICE_ENV_INPUT_CHANGED')
    source_config = validate_config(read_json(directory / 'input.json'))
    require(all(sha(read_bytes(source_config['source_env_files'][s])) == h for s, h in prepared['source_env_sha256'].items()), 'PREVIOUS_GENERATED_ENV_CHANGED')
    location = phase(directory, 'activate', {'generation': generation, 'target_receipt_sha256': prepared['target_receipt_sha256']})
    runner = runner_factory(location / 'commands')
    observed = {kind: inspect(runner, kind, receipt['targets'][kind]) for kind in KINDS}
    cluster_proof(runner, generation)
    require(platform_state(runner, receipt['targets']['platform']['database']) == before, 'PLATFORM_BEFORE_IMAGE_CHANGED')
    write_new(location / 'before.json', {'targets': observed, 'platform_sha256': sha(before)})
    runner.sql('postgres', 'BEGIN; SELECT pg_advisory_xact_lock(823140620261025);\n' + create_roles_sql(generation, passwords) + "\nSELECT jsonb_build_object('roles_created',4); COMMIT;", True)
    for kind in KINDS:
        item = phase(location, kind, {'target': receipt['targets'][kind]})
        sql = 'BEGIN; SET LOCAL standard_conforming_strings=on; SET LOCAL search_path=pg_catalog; SELECT pg_advisory_xact_lock(823140620261025);\n' + mutation_guard(receipt['targets'][kind]) + '\n' + schema_guard(kind, observed[kind]['schema_material']) + '\n'
        if kind == 'platform':
            sql += rotate_sql(before, hashes) + '\n'
        sql += grants_sql(kind, generation, observed[kind]['tables'])
        sql += "\nSELECT jsonb_build_object('transaction_ready',true); COMMIT;"
        result = runner.sql(receipt['targets'][kind]['database'], sql, True)
        acl = privilege_proof(runner, kind, generation, observed[kind]['tables'])
        write_new(item / 'after.json', {'committed_at': now(), 'result': result, 'privileges': acl})
    after = platform_state(runner, receipt['targets']['platform']['database'])
    require(after == rotate_expected(before, hashes), 'CREDENTIAL_ROTATION_CHANGED_UNAUTHORIZED_FIELDS')
    proof = cluster_proof(runner, generation, {}, False)
    write_new(location / 'after-platform.private.json', after)
    write_new(location / 'role-proof.private.json', proof)
    complete = {'status': 'ACTIVE_EPOCH_ROLES_NOLOGIN', 'generation': generation, 'completed_at': now(),
                'before_platform_sha256': sha(before), 'after_platform_sha256': sha(after),
                'epoch_sha256': prepared['epoch_sha256'], 'roles': {r['name']: r['oid'] for r in proof['roles']}}
    write_new(location / 'complete.json', complete)
    return {'status': complete['status'], 'generation': generation, 'users_incremented_once': len(before['users']), 'confidential_clients_rotated': 12, 'runtime_roles_nologin': 4}


def admission(directory, enable, runner_factory=Runner):
    prepared, receipt = load_journal(directory)
    generation = prepared['generation']
    activated = read_json(directory / 'activate/complete.json')
    require(activated['status'] == 'ACTIVE_EPOCH_ROLES_NOLOGIN' and activated['epoch_sha256'] == prepared['epoch_sha256'], 'COMPLETED_ACTIVATION_REQUIRED')
    name = 'enable-login' if enable else 'fence-login'
    if not enable:
        require(read_json(directory / 'enable-login/complete.json')['status'] == 'RUNTIME_LOGIN_ENABLED', 'PRIOR_LOGIN_ENABLE_REQUIRED')
    location = phase(directory, name, {'generation': generation, 'roles': activated['roles']})
    runner = runner_factory(location / 'commands')
    observed = {kind: inspect(runner, kind, receipt['targets'][kind]) for kind in KINDS}
    acls = {kind: privilege_proof(runner, kind, generation, observed[kind]['tables']) for kind in KINDS}
    write_new(location / 'target-proof.private.json', {'targets': observed, 'privileges': acls})
    if enable:
        require(platform_state(runner, receipt['targets']['platform']['database']) == read_json(directory / 'activate/after-platform.private.json'), 'PREOPEN_PLATFORM_STATE_CHANGED')
    before = cluster_proof(runner, generation, activated['roles'], not enable)
    write_new(location / 'before.json', before)
    runner.sql('postgres', admission_sql(activated['roles'], enable), True)
    after = cluster_proof(runner, generation, activated['roles'], enable)
    write_new(location / 'after.json', after)
    status = 'RUNTIME_LOGIN_ENABLED' if enable else 'RUNTIME_LOGIN_FENCED'
    write_new(location / 'complete.json', {'status': status, 'generation': generation, 'completed_at': now(), 'roles': activated['roles']})
    return {'status': status, 'generation': generation, 'runtime_roles': 4}


def admission_sql(roles, enable):
    guards, updates = [], []
    for role, oid in roles.items():
        guards.append('EXISTS(SELECT FROM pg_roles WHERE oid=' + str(oid) + ' AND rolname=' + literal(role) + ' AND rolcanlogin=' + ('false' if enable else 'true') + ')')
        updates.append('ALTER ROLE ' + ident(role) + (' LOGIN;' if enable else ' NOLOGIN;'))
    return "BEGIN; SELECT pg_advisory_xact_lock(823140620261025); " + do_sql('IF NOT(' + ' AND '.join(guards) + ") THEN RAISE EXCEPTION 'U06 role identity changed'; END IF;") + '\n' + '\n'.join(updates) + "\nSELECT jsonb_build_object('admission_changed',true); COMMIT;"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    for action in ('check', 'prepare', 'activate', 'enable-login', 'fence-login'):
        actions.add_argument('--' + action, action='store_true')
    parser.add_argument('--config')
    parser.add_argument('--journal')
    args = parser.parse_args(argv)
    require(sys.platform == 'linux' and os.geteuid() == 0, 'LOCAL_LINUX_ROOT_REQUIRED')
    require('microsoft' in os.uname().release.lower(), 'LOCAL_WSL_REQUIRED')
    if args.check or args.prepare:
        require(args.config and not args.journal, 'CONFIG_ONLY_REQUIRED')
        config = validate_config(read_json(args.config))
        validate_receipt(read_json(config['target_receipt_file']), config['generation'])
        if args.check:
            result = {'status': 'OFFLINE_CONTRACT_CHECKED_NO_DATABASE_ACCESS', 'generation': config['generation'], 'targets': 3, 'runtime_roles': 4}
        else:
            result = prepare(config)
    else:
        require(args.journal and not args.config, 'JOURNAL_ONLY_REQUIRED')
        directory = checked(args.journal)
        result = activate(directory) if args.activate else admission(directory, args.enable_login)
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (Invalid, OSError, ValueError, KeyError, TypeError) as error:
        print(json.dumps({'status': 'FAILED_QUARANTINE_RETAINED', 'error': str(error) if isinstance(error, Invalid) else 'PRIVATE_EPOCH_OPERATION_FAILED'}), file=sys.stderr)
        raise SystemExit(1)
