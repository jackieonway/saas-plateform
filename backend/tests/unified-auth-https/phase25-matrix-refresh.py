"""Rebuild reviewed Phase25 P/U/S matrix facts without promoting runtime gates.

The immutable preparation file is the input. Named executions, source bytes and
specific audited assertions are required; a suite total is never row coverage.
X/B/R and unresolved cleanup stay pending/blocked until separately reviewed.
"""
import argparse
from collections import Counter
from datetime import datetime, timezone, timedelta
import hashlib
import fnmatch
import json
from pathlib import Path
import re


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def need(value, code):
    if not value:
        raise RuntimeError('PHASE25_MATRIX_' + code)


class Refresh:
    def __init__(self, root, build_label):
        self.root = root.resolve(strict=True)
        self.validation = self.root / 'saas-plateform/backend/docs/designs/unified-auth/validation'
        self.prepared = self.validation / 'phase-25-matrix-preparation.json'
        need(digest(self.prepared) == '4eb1512cf92285fac95cc8e1fd9baf07e026dd72138e394812a8a960f7d3a7f2', 'PREPARATION_CHANGED')
        self.matrix = self.read(self.prepared.name)
        self.mapping = self.read('phase-25-component-execution-map.json')
        self.identity = self.read('phase-25-identity-components.json')
        self.platform = self.read('phase-25-platform-final-components.json')
        self.frontend = self.read('phase-25-frontend-components.json')
        self.dom = self.read('phase-25-identity-dom.json')
        self.protocol = self.read('phase-25-protocol-review-fixes-r2.json')
        self.now = datetime.now(timezone.utc).isoformat()
        self.reports = {}
        self.build_label = build_label
        self.candidate_report = 'phase-25-candidate-build-apps-r3.json' if build_label == 'phase25-apps-r3' else 'phase-25-candidate-build.json'
        candidate = self.read(self.candidate_report)
        need(candidate['status'] == 'PASS' and candidate['buildSnapshot']['label'] == build_label
             and candidate['buildSnapshot']['includeAllProjects'] and candidate['buildSnapshot']['completeFrontendOutputs'], 'CANDIDATE_BUILD_STATUS')
        need(self.identity['status'] == self.platform['status'] == self.frontend['status'] == 'PASS', 'COMPONENT_STATUS')
        need(self.identity['totals']['passed'] == 142 and self.platform['coverage']['uniquePassedTestNames'] == 432
             and not self.platform['coverage']['unresolvedFailures'], 'COMPONENT_COUNTS')
        need(self.dom['result'] == 'PASS' and self.dom['execution']['passed'] == 13
             and self.protocol['validation']['passed'] == 11 and self.protocol['validation']['failed'] == 0, 'DOM_OR_SDK_STATUS')
        requirements = {}
        audit = (self.validation / 'phase-25-evidence-layer-audit.md').read_text(encoding='utf-8')
        for identifier, layers in re.findall(r'\| `(P25-C\d\d-S\d\d)`[^|]*\| ([^|]+) \|', audit):
            requirements[identifier] = layers.strip().split('+')
        need(len(requirements) == 111, 'LAYER_AUDIT_INCOMPLETE')
        for row in self.matrix['scenarios']:
            row['requiredEvidenceLayers'] = requirements[row['id']]
            row['phase25Result'] = 'PENDING'
            row['actualEvidence'] = []
            row['componentFactsResult'] = 'PENDING'
            row['remainingAssertions'] = []
            row['executedAtUtc'] = None
            row['limitations'] = []
            row['relatedNamedExecutions'] = [entry for ref in row['componentRefIds']
                for entry in self.mapping['sourceReferenceExecutionMatches'].get(ref, [])]
        self.rows = {row['id']: row for row in self.matrix['scenarios']}

    def read(self, name):
        return json.loads((self.validation / name).read_text(encoding='utf-8-sig'))

    def register(self, name):
        self.reports[name] = {'sha256': digest(self.validation / name)}
        return self.reports[name]['sha256']

    def source(self, relative, expected):
        path = self.root / relative
        need(path.is_file() and digest(path) == expected, 'SOURCE_BYTES_CHANGED')
        return path

    def run_window(self, report, name):
        for run in report.get('runs',[]):
            if any(case['name']==name and case['result']=='OK' for case in run.get('cases',[])):
                started=run.get('startedAtUtc')
                if started and isinstance(run.get('durationSeconds'),(int,float)):
                    return {'startedAtUtc':started,'finishedAtUtc':(datetime.fromisoformat(started)+timedelta(seconds=run['durationSeconds'])).astimezone(timezone.utc).isoformat()}
        return None

    def pg(self, ref, assertion, lines=None):
        matches = self.mapping['sourceReferenceExecutionMatches'].get(ref, [])
        need(matches, 'NAMED_EXECUTION_MISSING_' + ref)
        facts = []
        for match in matches:
            report_path = self.validation / match['evidenceFile']
            need(digest(report_path) == match['evidenceSha256'], 'EXECUTION_REPORT_CHANGED')
            report = self.read(match['evidenceFile'])
            name = match['testName']
            executed = any(case['name'] == name and case['result'] == 'OK'
                for run in report.get('runs', []) for case in run.get('cases', []))
            executed = executed or name in report.get('coverage', {}).get('testNames', [])
            need(executed, 'TEST_NOT_PASSED')
            self.source(match['sourcePath'], match['sourceSha256'])
            self.register(match['evidenceFile'])
            fact = {**match, 'coverageLayer': 'P', 'observedAssertion': assertion,
                'sourceRef': ref, 'result': 'PASS', 'doesNotProveActualServiceRuntime': True}
            fact['executionWindow']=self.run_window(report,name)
            if lines:
                fact['assertionLines'] = lines
            facts.append(fact)
        return facts

    def extra_platform(self, name, source, line, assertion, layer='UNIT'):
        need(name in self.platform['coverage']['testNames'], 'EXTRA_TEST_NOT_PASSED')
        expected = self.platform['sourceInputs'][source]
        self.source(source, expected)
        report = 'phase-25-platform-final-components.json'
        return [{'coverageLayer': 'P', 'layer': layer, 'testName': name,
            'evidenceFile': report, 'evidenceSha256': self.register(report),
            'sourcePath': source, 'sourceSha256': expected, 'assertionLines': [line],
            'observedAssertion': assertion, 'result': 'PASS',
            'compositionIsNotSingleGreenInvocation': True,'executionWindow':self.run_window(self.platform,name)}]

    def dom_fact(self, names, assertion):
        observed = {case['name'] for case in self.dom['execution']['actualFixtureReport']['checks'] if case['status'] == 'passed'}
        need(set(names).issubset(observed), 'DOM_CASE_NOT_PASSED')
        for relative, expected in self.dom['sourceSha256'].items():
            self.source(relative, expected)
        report = 'phase-25-identity-dom.json'
        return [{'coverageLayer': 'U', 'layer': self.dom['evidenceLayer'], 'testNames': names,
            'evidenceFile': report, 'evidenceSha256': self.register(report),
            'observedAssertion': assertion, 'result': 'PASS', 'realIssuerRuntime': False,
            'cleanupStatus': self.dom['cleanup']['status'],
            'executionWindow':{key:self.dom[key] for key in ['startedAtUtc','finishedAtUtc']}}]

    def frontend_fact(self, relative, names, assertion, coverage='U', layer='UNIT_JS'):
        expected = self.frontend['sourceInputs'][relative]
        self.source(relative, expected)
        project, _, suffix = relative.partition('/tests/')
        command_file = 'tests/' + suffix
        runs = [run for run in self.frontend['checks'] if run.get('project') == project
            and any(fnmatch.fnmatchcase(command_file, argument) for argument in run.get('command', [])) and run.get('status') == 'PASS'
            and run.get('testCounts', {}).get('fail') == 0 and run.get('testCounts', {}).get('skipped') == 0]
        need(runs, 'FRONTEND_CASE_FILE_NOT_EXECUTED')
        text=(self.root/relative).read_text(encoding='utf-8')
        templates=[next(value for value in match if value) for match in re.findall(r'test\([\'\"]([^\'\"]+)[\'\"]|test\(`([^`]+)`', text)]
        variables={name: re.findall(r"['\"]([^'\"]+)['\"]", values)
            for name,values in re.findall(r'for \(const (\w+) of \[([^\]]+)\]\)',text)}
        declared=set()
        for template in templates:
            expansion=[template]
            for variable,values in variables.items():
                expansion=[item.replace('${'+variable+'}',value) for item in expansion for value in values] if '${'+variable+'}' in template else expansion
            declared.update(expansion)
        need(set(names).issubset(declared), 'FRONTEND_NAMED_CASE_NOT_DECLARED')
        report = 'phase-25-frontend-components.json'
        return [{'coverageLayer': coverage, 'layer': layer, 'testNames': names,
            'sourcePath': relative, 'sourceSha256': expected,
            'evidenceFile': report, 'evidenceSha256': self.register(report),
            'command': runs[-1]['command'], 'executionProof': 'Named source test in exact executed file; entire file completed with zero failed/skipped cases.',
            'observedAssertion': assertion, 'result': 'PASS', 'realIssuerRuntime': False,
            'executionWindow':{key:runs[-1][key] for key in ['startedAtUtc','finishedAtUtc']}}]

    def sdk_fact(self, assertion):
        for relative, expected in self.protocol['sourceSha256'].items():
            self.source(relative, expected)
        report = 'phase-25-protocol-review-fixes-r2.json'
        return [{'coverageLayer': 'S', 'layer': self.protocol['validation']['evidenceLayer'],
            'testNames': ['real trusted HTTPS SDK exchange checks claims, primary auth_time window, signature and wrong issuer/audience; code replay is denied'],
            'evidenceFile': report, 'evidenceSha256': self.register(report),
            'observedAssertion': assertion, 'result': 'PASS', 'realIssuerRuntime': False,
            'transportIsControlledIssuerFixture': True,'reportedAtUtc':self.protocol['recordedAtUtc']}]

    def rp_claim_fact(self, negative, positive, assertion):
        name='phase-25-rp-audience-components.json'
        report=self.read(name)
        need(report['result']=='PASS' and report['passed']==4 and report['failed']==report['skipped']==0
             and report['cleanup']['status']=='PASS', 'RP_CLAIM_REPORT')
        cases={case['name']:case for case in report['checks']}
        for identifier in [negative,positive]:
            case=cases[identifier]
            need(case['status']=='PASS' and case['cryptographicSignatureVerified'] and case['otherClaimsPreserved'], 'RP_CLAIM_ASSERTION')
        need(cases[negative]['callbackStatus']==400 and not cases[negative]['sessionEstablished']
             and cases[positive]['callbackStatus']==303 and cases[positive]['sessionEstablished'], 'RP_CLAIM_SESSION')
        for relative,expected in report['sourceSha256'].items():
            self.source(relative,expected)
        return [{'coverageLayer':'S','layer':report['evidenceLayer'],'testNames':[negative,positive],
            'evidenceFile':name,'evidenceSha256':self.register(name),'observedAssertion':assertion,
            'result':'PASS','realIssuerRuntime':False,'signedResponseSubstitutionAtSdkPort':True,
            'allClaimsExceptTestedFieldPreserved':True,'signatureVerifiedWithFixtureJwks':True,
            'executionWindow':{key:report[key] for key in ['startedAtUtc','finishedAtUtc']}}]

    def consumer_fact(self, name, test_names, assertion):
        report=self.read(name)
        need(report['status']=='PASS' and report['snapshot']=='phase25-apps-r3'
             and report['totals']['failed']==report['totals']['ignored']==0, 'CONSUMER_REPORT')
        passed={case['name'] for run in report['runs'] for case in run.get('cases',[]) if case['result']=='OK'}
        need(set(test_names).issubset(passed), 'CONSUMER_NAMED_TEST')
        for relative,expected in report['sourceInputs'].items():
            self.source(relative,expected)
        return [{'coverageLayer':'P','layer':'CURRENT_CONSUMER_COMPONENT_AND_EXPLICIT_TASK_PG_HTTP_FIXTURE',
            'testNames':test_names,'evidenceFile':name,'evidenceSha256':self.register(name),
            'snapshot':report['snapshot'],'snapshotSha256':report['snapshotSha256'],
            'observedAssertion':assertion,'result':'PASS','actualMultiServiceRuntime':False,
            'executionWindow':self.run_window(report,test_names[0]),
            'roleLayerPreserved':'The isolated tests retain their actual fixture/role boundaries; no canonical LOGIN or actual TLS upgrade is inferred.'}]

    def row(self, identifier, evidence, complete=True, missing=(), notes=()):
        row = self.rows['P25-' + identifier]
        row['actualEvidence'] = evidence
        completed=[entry['executionWindow']['finishedAtUtc'] for entry in evidence if entry.get('executionWindow')]
        row['executedAtUtc'] = max(completed,key=lambda value:datetime.fromisoformat(value)) if completed else None
        row['reviewedAtUtc']=self.now
        row['componentFactsResult'] = 'PASS' if evidence else 'PENDING'
        row['remainingAssertions'] = list(missing)
        row['limitations'] = list(notes)
        required = row['requiredEvidenceLayers']
        runtime = any(layer in ('X', 'B', 'R', 'R*', 'A') for layer in required)
        observed = {entry['coverageLayer'] for entry in evidence}
        if runtime:
            row['phase25Result'] = 'PENDING'
            row['remainingAssertions'].append('Current full candidate ' + '+'.join(layer for layer in required if layer in ('X','B','R','R*','A')) + ' evidence is not promoted by this P/U/S refresh.')
        elif complete and set(required).issubset(observed):
            row['phase25Result'] = 'PASS'
        else:
            row['phase25Result'] = 'PARTIAL' if evidence else 'PENDING'
        row['remainingAction'] = '；'.join(row['remainingAssertions']) or '当前已列层级的直接断言已满足；完整候选及阶段总体门禁仍单独保留。'

    def apply_reviewed_facts(self):
        p = self.pg
        r = self.row
        d = self.dom_fact
        f = self.frontend_fact
        s = self.sdk_fact
        r('C01-S01', p('I04', 'FIRST_PARTY and approved THIRD_PARTY are registered/read through auth_clients and the same private handler.', [29,109,140])
          + p('PCAP', 'Canonical runtime verifies retired oauth/oidc registries are absent.', [69,71]))
        r('C01-S02', p('I04', 'Second independent client in one application is registered.', [147,157])
          + p('I08', 'Confidential and PUBLIC siblings have separate callbacks and successful exchanges; PUBLIC has zero credential rows.', [1642,1678]),
          False, ['Explicit successful same-application client credential/callback/code cross-isolation negative remains for current runtime or another direct case.'])
        r('C01-S03', p('I19', 'Registration and runtime revalidate current exact TEST/PRODUCTION application admission and approved scope.'))
        r('C01-S04', p('IRETIRED', 'Retired identity protocol and registry router paths are absent.')
          + p('PRETIRED', 'Retired platform protocol/registry router paths are absent for every deployment mode.'))
        r('C02-S01', p('I12', 'ACCOUNT_ONLY branch returns a code with zero selection/intent port calls.'))
        r('C02-S02', f('saas-examples/oidc-rp/tests/oidc.test.mjs',
            ['real HTTPS standard discovery, Basic, S256, signed ID Token and matching opaque UserInfo establish a separate session'],
            'Delivered independent RP establishes a session using standard libraries against a trusted controlled HTTPS issuer.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C03-S01', p('I08', 'Canonical configured issuer/discovery endpoints and signed iss validation agree.'))
        r('C03-S02', p('I08', 'Signed ID Token is validated with the current client audience.') + s('Actual SDK validates correct audience and jose rejects the actual signed fixture token against a wrong expected audience.'))
        r('C03-S03', p('I06', 'Persisted subject is opaque and application/environment sector-bound; same-sector clients converge. [206-241]')
          + p('I08', 'Issued ID Token sub differs from internal user UUID.', [1584,1593]))
        r('C03-S04', p('I08', 'Issued nonce equals the original authorization transaction nonce; wrong expected nonce rejects.', [1566,1585])
          + f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['standard client rejects actual HTTPS nonce failure without creating a session or disclosing upstream material'],
            'Controlled HTTPS issuer signs a wrong nonce; real standard RP callback returns 400 and no session.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C03-S05', p('I08', 'max_age=0 does not reuse a stale authentication session.', [1768,1806])
          + s('Actual SDK rejects a stale signed auth_time and the cases driver rejects an inaccurate claimed primary window.'), False,
          ['Positive comparison of signed auth_time to an actual current primary/MFA authentication fact is still required.'],
          ['Fixture auth_time is controlled issuer input, not a performed SaaS primary authentication.'])
        r('C03-S06', p('I08', 'ID Token and actual router UserInfo sub/profile agree.', [1618,1638]))
        r('C03-S07', p('I08', 'Exact client/callback and wrong confidential credentials are rejected.')
          + s('SDK rejects an exchange using a different client identity in the controlled issuer fixture.'), False,
          ['Direct current distinct registered-client code and ID Token client isolation must retain exact assertions; a fixture unknown-client denial alone is insufficient.'])
        r('C03-S08', f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['standard client rejects actual HTTPS issuer failure without creating a session or disclosing upstream material'],
            'Wrong callback iss is rejected by standard RP without committing a session.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE')
          + self.rp_claim_fact('incorrect-issuer-valid-signature-no-session','expected-issuer-same-other-claims-session-established',
            'Actual delivered RP callback rejects valid signature with only JWT iss wrong; the same claims with correct iss establish a session.'),
          notes=['The deliberate same-key signed JWT replacement occurs at the SDK test port, not in the actual Identity service.'])
        r('C03-S09', f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['standard client rejects actual HTTPS nonce failure without creating a session or disclosing upstream material'],
            'A validly signed wrong nonce reaches the real standard RP callback; response400 and authenticated=false.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C03-S10', self.rp_claim_fact('incorrect-audience-valid-signature-no-session','expected-audience-same-other-claims-session-established',
            'Valid same-key signed wrong-aud ID Token reaches actual delivered RP callback:400, no UserInfo, no session; only aud corrected gives303 and a session.'),
          notes=['Controlled signed response at SDK customFetch test port; actual Identity interoperability is not inferred.'])
        for number, assertion in [(1,'Same application/environment two different clients receive identical persisted subject.'),
            (2,'Different application/partner or environment yields different subject.'),
            (3,'Renaming trusted display data and rotating signing keys leave persisted subject unchanged.'),
            (4,'RETIRE command seals three subject links; old account resolution rejects and replacement user never inherits old sub.')]:
            r(f'C04-S{number:02}', p('I06', assertion, [206,365]))
        r('C05-S01', p('I08', 'Unregistered authorization callback returns400; no unsafe redirect.', [1395,1417])
          + p('ICALLBACK', 'Exact registered callback including query and issuer binding is required.'), False,
          ['Keep current token-endpoint cross-client/redirect-uri and extra-parameter negatives explicit; parser helper alone does not prove all listed requests.'])
        r('C05-S02', [], False, ['Current authorize request with code_challenge_method=plain must reject without issuing a code.'])
        r('C05-S03', [], False, ['Current authorize requests must explicitly reject missing, short, long and illegal-character challenge.'])
        r('C05-S04', p('I08', 'Wrong valid-format verifier returns400; the correct verifier then redeems the same code.', [1499,1520]))
        r('C05-S05', p('I08', 'Second redemption returns400 and the confidential grant count remains one.', [1520,1545]))
        r('C05-S06', p('I22CODE', 'Independent AppState nodes block on real PG lock; one code creates one grant/consume/audit.'), notes=['Same process independent states are not two OS Identity processes. Preparation added that stronger runtime wording; it is not silently removed.'])
        r('C06-S01', p('I10', 'Refresh rotates and preserves original source/identity.') + p('PREFRESH', 'BFF cross-instance refresh has one winner and unknown result never replays.'))
        r('C06-S02', p('I22REFRESH', 'Real PG independent-state race yields one winner, reuse closes exact family and preserves other source.'), notes=['No additional OS process claim.'])
        r('C06-S03', p('I10REPLAY', 'Consumed refresh replay revokes live family, successor access/refresh and does not hide behind scope/old-token expiry.')
          + p('I22REFRESH', 'Concurrent reuse stays in its exact source family.'))
        r('C06-S04', p('I10', 'Actual family absolute deadline equals original source cap; expired absolute deadline rejects.', [2750,2761,2991,3007])
          + p('PDEADLINE', 'Actual post-network row lock cannot commit beyond original local/online deadline.'))
        r('C06-S05', p('I10', 'Online30min/offline24h idle windows and expired idle rejection are asserted on actual PG rows.', [2890,2956]))
        r('C06-S06', p('I10', 'Requested scope narrows the new access token within the original grant; authoritative scope changes are independently enforced.'))
        r('C06-S07', p('I10', 'Refresh request for unapproved scope is invalid_grant; current authority scope shrink is respected.'))
        r('C06-S08', p('I10', 'Offline explicitly approved grant survives natural browser expiry but GLOBAL logout/account disable revoke it; online remains source-capped.'))
        r('C07-S01', p('I08', 'PUBLIC client uses client_id without secret and has zero credential rows.', [1642,1678])
          + p('I04', 'PUBLIC secret creation/rotation is rejected.', [434,458])
          + f('saas-plateform/frontend/tests/oauth-client-policy.mjs', ['unified client form accepts supported USER, browser, native and MACHINE contracts'],
            'Actual exported form functions map SPA/NATIVE to PUBLIC and token auth none.', 'U', 'UNIT_JS_COMPONENT_EXTRACTION'), False,
          ['Current actual DOM still needs a PUBLIC scenario proving secret controls are absent and no secret is submitted.'],
          ['Form function execution is not actual browser DOM observation.'])
        r('C07-S02', p('I08', 'Wrong confidential USER secret returns401.', [1498]) + p('I11', 'Wrong/revoked MACHINE and inspector credentials reject.'), False,
          ['Explicit absent Basic credential negative must be named separately from wrong-secret and PUBLIC client branches.'])
        r('C07-S03', p('I04', 'Versioned overlap and fixed credential deadlines are enforced.', [400,432])
          + p('I11', 'New version2 secret issues token; emergency revocation rejects new and overlapping old secret and both token versions.', [2470,2538]))
        r('C07-S04', p('I11', 'Emergency revoke advances policy, rejects both credential versions and deactivates their tokens.'))
        r('C08-S01', p('I09', 'ID Token sent to actual introspection yields active=false; UserInfo rejects it as Bearer.', [3373,3398]))
        store_audience=['authorization::tests::cross_application_tokens_and_malformed_resource_introspection_are_rejected',
            'authorization::tests::resource_reply_rejects_wrong_authority_subject_scope_and_original_binding']
        order_audience=['authorization::tests::resource_introspection_rejects_wrong_audience_activity_type_identity_and_expiry']
        r('C08-S02', p('I09', 'Wrong resource audience and unrelated resource client introspection are inactive.')
          + p('PRESOURCE', 'Actual platform resource handler rejects wrong audience in online proof.', [302,309])
          + self.consumer_fact('phase-25-stores-components.json',store_audience,'Current repaired stores consumer rejects wrong app/audience proof before data access.')
          + self.consumer_fact('phase-25-orders-components.json',order_audience,'Current repaired work-orders consumer rejects unrelated audience/activity/type/expiry.'))
        r('C08-S03', p('I09', 'Actual resource and scope tuple bounds access; resource caller cannot introspect unrelated audience.')
          + p('PRESOURCE', 'Wrong online scope is rejected by actual canonical resource HTTP handler.')
          + self.consumer_fact('phase-25-stores-components.json',['authorization::tests::exact_resource_action_permission_and_single_identity_bound_grant_are_required'],
            'Current stores consumer requires exact resource/action permission plus independent identity-bound grant.')
          + self.consumer_fact('phase-25-orders-components.json',['canonical_user_requires_explicit_scope_binding_and_private_principal'],
            'Current work-orders PG HTTP request rejects missing deployed scope binding and wrong principal.'))
        r('C08-S04', p('PRESOURCE', 'Wrong subject mode/member proof and MACHINE personnel directory access reject.')
          + self.consumer_fact('phase-25-stores-components.json',['authorization::tests::canonical_user_principal_is_separate_from_the_public_opaque_subject',
            'authorization::tests::context_identity_expiry_and_hard_ceiling_fail_closed'], 'Current stores consumer validates USER private principal and exact original context tuple.')
          + self.consumer_fact('phase-25-orders-components.json',['canonical_machine_cannot_enter_person_directory_or_assignment',
            'canonical_machine_and_unbound_user_cannot_create_person_audits'], 'Current work-orders actual PG handlers reject MACHINE person directory/assignment/audit and unbound USER.'))
        r('C08-S05', p('I11', 'MACHINE token has no person subject/auth_time or refresh/ID Token; PUBLIC USER machine grant rejects.')
          + p('PRESOURCE', 'Approved machine response contains no principal/context/sub and personnel directory rejects MACHINE.')
          + self.consumer_fact('phase-25-orders-components.json',['canonical_machine_read_export_respect_only_explicit_delegation',
            'canonical_machine_cannot_enter_person_directory_or_assignment'], 'Actual work-orders PG HTTP MACHINE read/export requires explicit delegation; person directory/assignment denied.'))
        r('C09-S01', p('I07CONSENT', 'Missing approval/scope coverage prevents issuance; approval is version-bound.')
          + d(['single-organization-enters-after-first-consent'], 'Real DOM shows permissions and records consent before approval.'), False,
          ['Direct DOM assertions of application/purpose/resource/scope text are not present in the current case.'])
        r('C09-S02', p('I07CONSENT', 'Unchanged group generation and scope coverage reuse prior consent.')
          + d(['remembered-single-identity-needs-no-clicks','remembered-approval-survives-reload-search-does-not-auto-select'], 'Remembered approval hides permissions and does not submit another consent.'))
        r('C09-S03', p('I07CONSENT', 'Expanded scope/group version requires reapproval; old consent cannot cover new members.')
          + p('I08', 'Expanded consent enters CONSENT_REQUIRED.', [1418,1459]), False,
          ['Actual login DOM case must assert re-prompt for newly added resource/scope, not only a management-page warning.'])
        r('C09-S04', p('I07CONSENT', 'Different group/application/environment cannot inherit consent.'))
        r('C09-S05', p('I07GROUP', 'Only explicit context/app/environment-bound group members can join; changes advance group version.')
          + p('I07CONSENT', 'Existing approval cannot silently apply to newly changed group membership.'))
        r('C10-S01', p('I07DELETE', 'Active consent DELETE is409; subsequent durable source/generation invariants remain intact.', [326,337]))
        r('C10-S02', p('I07DELETE', 'Revoke atomically closes pending codes, access, refresh, grant and outbox without crossing group/account.', [360,390])
          + p('PCAP', 'Revoked grant/SSO/BFF and other original authority shrink disable the exact derived capability.'))
        r('C10-S03', p('I07DELETE', 'Display deletion keeps revoked generation tombstone and does not revive the original grant.', [399,414]))
        r('C10-S04', p('I07DELETE', 'After new authorization version4, old expectedVersion2 DELETE conflicts and new active authorization survives.', [430,452]), notes=['Explicit ordered stale-command test; no OS concurrency is claimed.'])
        for suffix, names, assertion in [
            ('01',['no-members-never-auto-approves'],'Zero legal candidates never auto-approve; forged selection is rejected by the actual server-side selection checks.'),
            ('02',['single-tenant-skips-tenant-step'],'One tenant with multiple organizations skips tenant UI and still requires explicit organization.'),
            ('03',['multiple-tenants-and-organizations-require-choice'],'Select tenant before organizations; no first item is implicitly chosen.'),
            ('04',['single-organization-enters-after-first-consent','tenant-with-one-organization-enters-directly'],'Exactly one valid selected-tenant member continues with its precise membership ID.'),
            ('05',['multiple-tenants-and-organizations-require-choice'],'Multiple members require explicit choice; server tuple remains account/client/member/version-bound.'),
            ('06',['remembered-single-identity-needs-no-clicks'],'Remembered consent and exactly one candidate produce approve without another confirmation click.')]:
            r('C11-S'+suffix, p('I12', assertion) + d(names, assertion))
        r('C11-S07', d(['remembered-approval-survives-reload-search-does-not-auto-select'], 'Filtering to one radio leaves posts=[] and never auto-approves.'))
        r('C11-S08', p('PSELECT', 'Customer purpose and exact BFF/member/source bindings are enforced by actual platform transactions.')
          + p('PRESOURCE', 'Employee/customer/member/subject modes cannot substitute in resource proof.'))
        r('C11-S09', p('I12', 'ACCOUNT_ONLY branch observes no intent/selection calls.'))
        for suffix, assertion in [('01','Organization/tenant status and organization admission shrink reject the original capability; regrant never revives it.'),
            ('02','Exact membership status/version shrink rejects the original capability without touching sibling identities.'),
            ('03','Application status/policy and tenant/organization admission changes invalidate the original app capability in the transaction.'),
            ('05','After admission is removed and reinserted, original capability stays revoked and revocation cannot be reversed.')]:
            r('C12-S'+suffix, p('PCAP', assertion, [74,101]) + p('PBIND' if suffix=='02' else 'PAPP', assertion))
        r('C12-S04', p('PRESOURCE', 'A permitted OAuth scope without an independent current data grant returns403.', [313,321]))
        r('C13-S01', p('PSELECT', 'Same operation result bytes/capability are recovered without extending/recreating context; this is one platform DB plus explicit source port.'))
        r('C13-S02', p('PSELECT', 'Repeated result equals original response and customer context count stays1; second consume conflicts.', [187,206])
          + p('I12', 'Context result delivery is one-time and source/version-bound.'))
        r('C13-S03', f('saas-plateform/frontend/tests/micro-context-updates.mjs', ['authorization, membership, complete entry tuple and deadline changes still cancel the prior snapshot'],
            'Current host service aborts previous snapshot and advances epoch for every authority/context/entry change.'))
        r('C13-S04', p('PLATE', 'Late logout/callback cannot erase the newly selected account Cookie.')
          + f('saas-plateform/frontend/tests/notification-context.mjs', ['old HTTP body and old HTTP errors cannot cross into a new organization when transport ignores AbortSignal'],
            'Late success/error ignoring AbortSignal never enters the new organization snapshot.')
          + f('saas-plateform/frontend/tests/bff-integration.mjs', ['a BFF member switch retires an actual API response even when transport ignores abort'],
            'Current API owner invalidates a late response after BFF member switch.'))
        r('C13-S05', p('PLATE', 'Late Cookie mutation is fenced in canonical PG; no actual same-browser two-tab runtime is claimed.'))
        r('C13-S06', f('saas-plateform/frontend/tests/micro-context-updates.mjs', ['authorization, membership, complete entry tuple and deadline changes still cancel the prior snapshot'],
            'Current host context lifecycle cancels old signal and increments epoch.'))
        r('C13-S07', p('NOTIFY', 'Real notification PG/HTTP fixture validates replacement/revocation/reconnect and exact recipient identity; current full-service socket still separate.'))
        r('C14-S01', p('PPERMS', 'Removing VIEW fine permission denies security read; REGISTER/REVIEW revocation denies their exact mutations.'), False,
          ['A directly named VIEW-only read plus all write/review/rotate negatives is not inferred from a full-permission actor.'])
        r('C14-S02', p('PREVIEW', 'Applicant/registrar/submitter cannot approve their own request; requests preserve actor/version.')
          + p('PPERMS', 'REGISTER/REVIEW fine grants and current org scope control writes.')
          + self.extra_platform('oidc::remote_admin::pg_tests::phase19_disable_role_uses_actual_private_owner_and_rejects_hidden_scope_changes',
            'saas-plateform/backend/src/oidc/remote_admin_pg_tests.rs', 596, 'With SCOPES menu disabled, exact stop-only command commits a DISABLE intent bound to actual app owner; hidden metadata/callback or forged app owner is forbidden without partial intent.', 'CANONICAL_PG')
          + self.extra_platform('oidc::remote_admin::tests::phase19_disable_only_authority_cannot_hide_metadata_callback_or_scope_changes',
            'saas-plateform/backend/src/oidc/remote_admin_tests.rs', 725, 'Actual stop_only predicate rejects permissions scope changes and duplicate scopes in addition to hidden metadata; this part is a unit assertion.')
          + self.extra_platform('auth_review::tests::phase19_canonical_assignment_is_explicit_and_original_source_revocation_fences_writes',
            'saas-plateform/backend/src/auth_review/tests.rs', 542, 'Exact app assignment gates allowed REGISTER actor and forbids unassigned app/source; independent approval and publication remain separate.', 'CANONICAL_PG'),
          False, ['A directly named positive authorized SCOPES metadata/permission write still needs exact evidence; current stop-only scope negatives cannot rule out a reject-all SCOPES defect.'],
          notes=['SCOPES field predicate unit and actual PG permission boundary are composed explicitly; no single browser-role matrix run is claimed.'])
        r('C14-S03', p('PREVIEW', 'Self approval and stale/incorrect evidence conflict.')
          + self.extra_platform('auth_review::tests::phase19_canonical_configured_production_requires_distinct_bound_publishers',
            'saas-plateform/backend/src/auth_review/tests.rs', 710, 'Independent reviewer and bound publisher persist distinct actor IDs; applicant and unbound publisher are rejected.', 'CANONICAL_PG')
          + self.extra_platform('auth_review::tests::phase19_canonical_production_default_and_missing_binding_remain_closed',
            'saas-plateform/backend/src/auth_review/tests.rs', 425, 'Default/missing responsibility binding remains productionReady=false and publish is forbidden.', 'CANONICAL_PG'),
          notes=['Responsibility bindings and completed remote replies here are explicit test data, not actual Phase26 release personnel or synchronization.'])
        r('C14-S04', p('IKEYPERM', 'KEY_ADMIN command requires exact actor/intent/fresh TOTP, service purpose and auditable receipt.')
          + p('I04', 'Wrong service purpose cannot administer clients; PUBLIC cannot obtain a secret.'), False,
          ['Keep explicit no-ROTATE client-secret and service-secret permission negatives separate from KEY_ADMIN.'])
        r('C14-S05', p('PPERMS', 'After actual role/fine grant withdrawal, new command and original-operation replay both forbid.')
          + self.extra_platform('auth_review::tests::phase19_canonical_assignment_is_explicit_and_original_source_revocation_fences_writes',
            'saas-plateform/backend/src/auth_review/tests.rs', 542, 'Unassigned application and original source revocation cannot write through an otherwise allowed role.', 'CANONICAL_PG'))
        r('C14-S06', p('I07DELETE', 'Missing CSRF and foreign Origin consent mutations reject without durable mutation.')
          + f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['Origin/CSRF, duplicate callback, unapproved field and fixed-host checks reject before a token exchange'],
            'Actual standard RP rejects Origin/CSRF, duplicate callback state and Host before token exchange.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'), False,
          ['Do not infer platform/SSO duplicate-Cookie and duplicate-parameter mutation cases from RP callback state only.'])
        r('C14-S07', p('PAUDIT', 'Application/menu cascades atomically roll back if audit write fails.')
          + p('PVERSION', 'Actual audit/optimistic conflict failures leave application policy unchanged.')
          + p('I07DELETE', 'Silent audit INSERT suppression leaves consent, receipt/outbox and grant state unchanged.'))
        r('C14-S08', p('I04', 'Same operation recovers its receipt and mismatched input conflicts without persisting one-time secret.')
          + p('PREVIEW', 'Same reviewed operation restores exact original result and audit count stays fixed.'))
        r('C14-S09', p('PREVIEW', 'Stale expected version and wrong evidence cannot overwrite current approval.')
          + p('PVERSION', 'Actual concurrent policy conflicts preserve versions and atomic audit.')
          + p('I07DELETE', 'Stale DELETE cannot cross newly authorized consent version.'))
        for suffix, ref, assertion in [('01','ILOGOUT','CURRENT_APP only closes original grant and preserves other RP/device and consent.'),
            ('02','ILOGOUT','GLOBAL closes original account USER/SSO/offline sources and preserves consent.'),
            ('03','I07DELETE','Group consent revoke closes exact derived sources and preserves unrelated group/account.')]:
            r('C15-S'+suffix, p(ref, assertion))
        r('C15-S04', p('ISIGNED', 'Actual worker signing payload is bound to original RP target generation.')
          + p('PBACK', 'Valid canonical back-channel closes original sid/sub and does not revoke newer source.')
          + f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['valid signed backchannel closes only the exact sid/sub, rejects replay and cannot revive through refresh']
            + [f'backchannel rejects {kind} and preserves the unrelated authenticated session' for kind in ['audience','issuer','signature','nonce','event','eventBoolean','expired','mismatch','header']],
            'Actual independent RP receiver validates signed audience, issuer, signature, event and original opaque sid/sub; replay cannot reopen.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C15-S05', p('PBACK', 'Unknown sid tombstone before callback persists a durable fence and cannot revoke a new source.')
          + f('saas-examples/oidc-rp/tests/capacity.test.mjs', ['source-full signed logout closes the exact local source without acknowledging or evicting earlier facts'],
            'Receiver preserves original source/tombstone/replay facts under capacity.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE')
          + f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['signed unknown-sid tombstone blocks a late callback while UserInfo is waiting',
            'signed backchannel racing refresh wins before the result can reattach the original source'],
            'Actual independent RP receiver tombstone wins against callback/UserInfo and refresh already in flight; no local source can reattach.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C15-S06', p('IDELIVERY', 'Concurrent delivery leases, retry budget and stale completion are bounded and preserve original operation.'))
        r('C15-S07', p('PDEADLINE', 'Original ID/access/local deadlines cannot be extended by row-lock or refresh timing.')
          + f('saas-examples/oidc-rp/tests/oidc.test.mjs', ['refresh rotates once, keeps the original opaque identity, and failure closes the local session'],
            'Standard RP retains original local expiry and closes uncertain/failed refresh; actual no-backchannel runtime window remains separate.', 'S', 'STANDARD_CLIENT_TLS_FIXTURE'))
        r('C16-S01', d(['single-tenant-skips-tenant-step'], 'Current identity DOM desktop uses real Chromium; actual four-service desktop flow not claimed.'))
        r('C16-S02', d(['mobile-single-tenant-multiple-organizations'], '390x844 current identity DOM has no horizontal overflow; actual complete mobile service flows remain separate.'))
        for suffix, assertion in [('03','Actual create/edit dialog traps keyboard Tab/Shift+Tab and Escape restores opener.'),
            ('04','Actual 390px dialog contains activeElement focus and Escape restores its valid trigger.'),
            ('05','Actual 390px long client form fits viewport with reachable actions and internal scroll.')]:
            focus = self.read('phase-25-ui-focus-fixture.json')
            need(focus['status'] == 'PASS' and focus['testSha256'] == digest(self.root / 'saas-plateform/frontend/tests/unified-clients-browser.mjs'), 'FOCUS_FIXTURE_CHANGED')
            evidence = [{'coverageLayer':'U','layer':'REAL_REACT_BROWSER_EXPLICIT_ROUTE_FIXTURE',
                'evidenceFile':'phase-25-ui-focus-fixture.json','evidenceSha256':self.register('phase-25-ui-focus-fixture.json'),
                'testNames':['390px editor contains keyboard focus and restores its opener on Escape','390px editor fits viewport and creates a valid machine client'],
                'observedAssertion':assertion,'result':'PASS','realIssuerRuntime':False}]
            missing={'03':['Keyboard login/consent/context Radio/Enter assertions remain outside this dialog-only fixture.'],
                '04':['Error-state focus recovery has no direct assertion in this dialog-only fixture.'],
                '05':['Long consent form scroll and action reachability is not inferred from the long client editor.']}
            r('C16-S'+suffix, evidence, False, missing[suffix])
        r('C16-S06', p('ICALLBACK', 'Exact callback and issuer parameter validation reject an unregistered destination.')
          + p('I12CALLBACK', 'Invalid client context never redirects to the unregistered callback.'), False,
          ['Browser must demonstrate malicious returnTo/restart does not navigate away; pure parser/transport fixture is not that DOM observation.'])
        r('C16-S07', d(['customer-and-hostile-names-render-as-text'], 'Hostile organization markup renders as text; zero injected img elements; production asset CSP is served.'), False,
          ['Direct application/scope/display-name hostile content and CSP execution-block assertions are not inferred from the single organization fixture.'])
        for suffix, refs, assertion in [('01',['POUTAGE'],'Unavailable authority does not execute business or revoke valid local state.'),
            ('02',['PRESOURCE'],'Platform/identity proof timeout or denial is unavailable and does not open business.'),
            ('03',['I22POOL'],'Closed canonical PG pool returns503; this is not a stopped PostgreSQL process.'),
            ('04',['I22POOL'],'Real saturated pool has bounded503 and preserves original tokens.'),
            ('05',['IQUEUE'],'Request admission/oversized body rejects before authority; live queue saturation still separate.'),
            ('06',['INET','PDEADLINE'],'Actual slow HTTP body expires and releases admission; late post-lock commit is rejected.'),
            ('08',['I22LOCK','PREFRESH'],'Lock timeout503 and uncertain refresh original-operation fences preserve legal sources.')]:
            r('C17-S'+suffix, [item for ref in refs for item in p(ref, assertion)])
        r('C17-S07', self.extra_platform('bff_tests::logout_signature_domain_and_original_short_claims_are_strict',
            'saas-plateform/backend/src/bff_tests.rs', 87, 'Production logout-token validator rejects iat+6 seconds, accepts+5, and rejects past/expired exact boundaries.')
          + p('PDEADLINE', 'Production ID/access deadline proof cannot commit after actual lock wait crosses expiry.'), False,
          ['Signed logout skew boundary is proven; ID Token future-iat/auth_time permitted and excessive skew boundaries need their own actual validator/S proof.'])
        r('C18-S01', p('IKEY', 'Current canonical PG lifecycle covers publication/activation/terminal/emergency state; actual runtime/JWKS cache is not promoted.'))
        r('C18-S02', [], False, ['Current real auth/portal restore-new/reconcile plus explicit business backup/restore boundaries remain required.'])
        r('C18-S03', [], False, ['Current real capacity/CPU/memory/queue and successful CODE/REFRESH/INTROSPECTION measurement remain required.'])
        r('C18-S04', p('I22RATE', 'Rotating socket sources cannot reset the exact shared client budget; current alarm delivery is separate.'))
        r('C18-S05', p('IREDACT', 'Current component metrics/password logging omits request secrets; actual current service/gateway/report log scan remains required.'))
        r('C18-S06', [], False, ['Automatic approval rejected targeted deletion before command startup; exact owned temporary files/junction remain.'])
        self.rows['P25-C18-S06']['phase25Result'] = 'BLOCKED'
        self.rows['P25-C18-S06']['cleanupEvidenceFiles'] = [name for name in (
            'phase-25-frontend-cleanup.json','phase-25-frontend-cleanup-retry.json','phase-25-identity-dom.json')
            if (self.validation/name).is_file()]

    def write(self):
        self.apply_reviewed_facts()
        need(len(self.rows) == 111, 'ROWS_LOST')
        runtime_rows = [row for row in self.rows.values() if any(layer in ('X','B','R','R*') for layer in row['requiredEvidenceLayers'])]
        need(all(row['phase25Result'] != 'PASS' for row in runtime_rows), 'RUNTIME_FALSE_PASS')
        counts = dict(Counter(row['phase25Result'] for row in self.rows.values()))
        for row in self.rows.values():
            row['preparationComponentEvidenceIds']=row.pop('componentEvidenceIds',[])
            row['componentEvidenceIds']=sorted({fact['evidenceFile'] for fact in row['actualEvidence']})
            row['relatedNamedExecutionsAreOnlyRetrievalHints']=True
        self.matrix.update({'documentRole':'CURRENT_LAYERED_ASSERTION_RESULTS_RUNTIME_GATES_OPEN',
            'overallResult':'PARTIAL', 'phaseCompletionClaimed':False,'runtimeExecutedByThisDocument':False,
            'refreshedAtUtc':self.now, 'sourceBaseline':{
                'candidateBuildLabel':self.build_label,'componentInputLabel':'phase25-apps-r2',
                'candidateBuildEvidence':{'file':self.candidate_report,'sha256':self.register(self.candidate_report)},
                'componentSnapshotSha256':self.identity['snapshotSha256'],
                'fullCandidateRuntimeValidated':False,
                'domainCombination':'Identity/platform/frontend facts use exact current source equality to apps-r2. Only stores/work-orders lib.rs changed in apps-r3; their current 98/149 tests and source/binary manifests are cited separately. No full runtime equivalence is inferred.',
                'platformEvidence':'432 unique final PASS names are composed from current separate runs; original408PASS/1FAIL and exact TLS1PASS/431filtered are preserved, never one green run.',
            },'resultCounts':counts,'currentEvidenceReports':self.reports,
            'preparationPreserved':{'json':'phase-25-matrix-preparation.json','sha256':digest(self.prepared),'markdown':'phase-25-matrix-preparation.md'},
            'coveragePolicyReferences':['phase-25-evidence-layer-audit.md','phase-25-assertion-gap-audit.md'],
            'priorEvidenceRole':'PRESERVED_PREPARATION_HISTORY_ONLY_NOT_CURRENT_ACCEPTANCE',
            'sourceRefsRole':'Preparation source locators are retained; current result proof uses actualEvidence source/report digests and named executed assertions.',
            'currentRuntimeRule':'No previous Phase24 or failed early Phase25 startup is promoted. X/B/R rows stay PENDING; full cleanup is BLOCKED.'})
        for category in self.matrix['categories']:
            states = [self.rows[identifier]['phase25Result'] for identifier in category['scenarioIds']]
            category['phase25Result'] = 'PASS' if all(state=='PASS' for state in states) else 'PARTIAL' if any(state in ('PASS','PARTIAL') for state in states) else 'BLOCKED' if 'BLOCKED' in states else 'PENDING'
            category['resultCounts'] = dict(Counter(states))
        self.matrix['cleanup']['phase25Result'] = 'BLOCKED'
        self.matrix['cleanup']['reason'] = 'Actual temporary-directory deletion rejected by automatic approval before command startup; retained resources are listed in independent current cleanup reports.'
        output = self.validation/'phase-25-matrix.json'
        output.write_text(json.dumps(self.matrix,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        markdown = ['# Phase 25 当前分层验收矩阵','',
            '状态：**PARTIAL，未完成 Phase 25**。本轮按当前命名测试和具体断言回填P/U/S；X/B/R当前全版本结果仍PENDING，完整清理BLOCKED。',
            '',f'更新：`{self.now}`。当前候选binary基线为`{self.build_label}`；身份/平台组件输入为`phase25-apps-r2`，逐源码摘要核对。门店98/工单149已用修复后的apps-r3独立执行，其他域只按精确输入等价组合，完整候选运行态仍未通过。',
            '', '平台432是分层合成的唯一最终PASS集合；保留原408 PASS/1 FAIL、同binary定向TLS1 PASS/431 filtered与integration23 PASS，不声称一次432全绿。',
            '', '依据：[层级独审](phase-25-evidence-layer-audit.md)、[逐断言审计](phase-25-assertion-gap-audit.md)。原始准备稿保留为[JSON](phase-25-matrix-preparation.json)与[Markdown](phase-25-matrix-preparation.md)，不抹去旧PENDING/失败历史。',
            '', 'P为具体生产代码/PG事实，S为标准库与受控有效签名响应，U保留真实DOM或明确前端组件本来的层级；它们均不冒充实际多服务HTTPS/浏览器/恢复。相关sourceRef匹配只是检索线索，actualEvidence才列经过断言审查的事实。',
            '', '## 当前计数','', '| 结果 | 子例 |','| --- | ---: |']
        markdown += [f'| {state} | {counts.get(state,0)} |' for state in ('PASS','PARTIAL','PENDING','BLOCKED')]
        for category in self.matrix['categories']:
            markdown += ['',f"## {category['id']} {category['title']}",'', '| 子例 | 必须观察 | 必要层级 / 结果 | 当前直接证据 | 未完成 / 限制 |','| --- | --- | --- | --- | --- |']
            for identifier in category['scenarioIds']:
                row=self.rows[identifier]
                evidence=[]
                for item in row['actualEvidence']:
                    source=item.get('sourcePath'); source_link=''
                    if source:
                        prefix='../../../../../../'; line=item.get('assertionLines',[1])[0]
                        source_link=f' [源码]({prefix}{source}#L{line})'
                    names=item.get('testNames',[item.get('testName','')])
                    evidence.append(f"[{item['evidenceFile']}]({item['evidenceFile']}) `{item['layer']}`: {item['observedAssertion']}"+source_link+'<br>'+('; '.join(names)))
                limits='；'.join(row['remainingAssertions']+row['limitations']) or '当前具体层级通过；不表示阶段全版本完成。'
                escape=lambda value:str(value).replace('|','\\|').replace('\n',' ')
                markdown.append('| '+ ' | '.join(escape(value) for value in (f"`{identifier}` {row['label']}",row['requiredObservation'],
                    '+'.join(row['requiredEvidenceLayers'])+' / **'+row['phase25Result']+'**','<br><br>'.join(evidence) or '尚无已审查的直接执行证据',limits))+' |')
        markdown += ['', '## 当前证据文件','', '| 文件 | SHA-256 |','| --- | --- |']
        markdown += [f'| [{name}]({name}) | `{value["sha256"]}` |' for name,value in sorted(self.reports.items())]
        markdown += ['', '## 保留门禁','',
            '- 真实当前全版本主链、标准客户端、业务与通知/SDK、密钥和恢复、容量与告警结果须独立引用，不接受早期startup FAIL或旧阶段报告替代。',
            '- Cleanup：自动审批审核在命令启动前拒绝定向删除，原因仅`blocked by policy`；未换shell绕过，当前临时目录与依赖junction的实际残留见各清理报告。',
            '- 本脚本不启动服务、不操作数据库/Docker、不删除资源；它只核验报告和源码摘要并回填经过人工审查的断言。',
            f'- 重现：`python -B backend/tests/unified-auth-https/phase25-matrix-refresh.py --workspace <workspace-root> --candidate-build-label {self.build_label}`。', '']
        (self.validation/'phase-25-matrix.md').write_text('\n'.join(markdown),encoding='utf-8')
        return {'result':'PARTIAL','rows':111,'resultCounts':counts,'matrixSha256':digest(output),'runtimeRowsPromoted':0}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace',type=Path,required=True)
    parser.add_argument('--candidate-build-label',default='phase25-apps-r2')
    args=parser.parse_args()
    need(re.fullmatch(r'phase25-apps-r[1-9][0-9]*',args.candidate_build_label), 'BUILD_LABEL')
    print(json.dumps(Refresh(args.workspace,args.candidate_build_label).write()))


if __name__=='__main__':
    main()
