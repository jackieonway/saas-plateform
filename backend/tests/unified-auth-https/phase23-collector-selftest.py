"""Pure parser/control tests; never start the actual collector or a listener."""
import base64
import hashlib
import json
import unittest

from collector import control_authorized, logout_evidence


CANARY = 'PRIVATE_LOGOUT_EVIDENCE_CANARY'
SID_HASH = hashlib.sha256(CANARY.encode()).hexdigest()
REQUEST = {'expectedKid': 'phase23-b', 'publicSidSha256': SID_HASH}


def segment(value):
    raw = value if isinstance(value, bytes) else json.dumps(value).encode()
    return base64.urlsafe_b64encode(raw).rstrip(b'=').decode()


def message(header=None, payload=None):
    # Deliberately no RSA signing in a metadata parser test. Actual delivery is
    # supplied only by the owner's worker/collector runtime, not this fixture.
    head = {'alg': 'RS256', 'typ': 'logout+jwt', 'kid': 'phase23-b'}
    body = {'sid': CANARY, 'sub': CANARY + '-subject', 'iat': 1000, 'exp': 1120,
            'events': {'http://schemas.openid.net/event/backchannel-logout': {}}}
    if header:
        head.update(header)
    if payload:
        body.update(payload)
    return {'kind': 'LOGOUT', 'body': {'logout_token': '.'.join(
        (segment(head), segment(body), segment(b'parser-only-signature')))}}


class EvidenceTests(unittest.TestCase):
    def test_independent_control_key_requires_exactly_one_matching_header(self):
        secret = 'a' * 64
        self.assertTrue(control_authorized([secret], secret))
        for values in ([], ['b' * 64], [secret, secret], None, [None]):
            self.assertFalse(control_authorized(values, secret))

    def test_only_fixed_safe_metadata_is_returned(self):
        value = logout_evidence([message(), message()], REQUEST)
        self.assertEqual(value, {'matched': True, 'count': 2, 'kid': 'phase23-b'})
        encoded = json.dumps(value)
        for private in (CANARY, SID_HASH, 'logout_token', 'sid', 'sub', 'claims'):
            self.assertNotIn(private, encoded)

    def test_wrong_kid_algorithm_type_sid_or_lifetime_does_not_match(self):
        entries = [message(header={'kid': 'owned-https'}), message(header={'alg': 'none'}),
                   message(header={'typ': 'JWT'}), message(payload={'sid': 'other'}),
                   message(payload={'exp': 1000}), message(payload={'iat': True}),
                   message(payload={'events': {}})]
        self.assertEqual(logout_evidence(entries, REQUEST), {'matched': False, 'count': 0, 'kid': None})

    def test_malformed_or_ambiguous_jwt_and_nonlogout_messages_are_ignored(self):
        duplicate = message()
        parts = duplicate['body']['logout_token'].split('.')
        parts[0] = segment(b'{"alg":"none","alg":"RS256","typ":"logout+jwt","kid":"phase23-b"}')
        duplicate['body']['logout_token'] = '.'.join(parts)
        self.assertEqual(logout_evidence([duplicate, {'kind': 'INVITATION', 'body': {}},
            {'kind': 'LOGOUT', 'body': {'logout_token': CANARY}}, None], REQUEST)['count'], 0)

    def test_query_is_exact_bounded_and_search_only_uses_the_last_thirtytwo_messages(self):
        for request in ({}, {**REQUEST, 'token': CANARY}, {**REQUEST, 'expectedKid': '../key'},
                        {**REQUEST, 'publicSidSha256': 'a' * 63}, []):
            with self.assertRaises(ValueError):
                logout_evidence([], request)
        self.assertEqual(logout_evidence([message()] + [{'kind': 'INVITATION'}] * 32, REQUEST)['count'], 0)


if __name__ == '__main__':
    unittest.main()
