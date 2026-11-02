"""Phase 22 private metrics aperture over the existing task TLS gateway.

The public gateway still denies every internal route, including with a valid
metrics token. The Identity endpoint separately enforces its operational token.
"""
import gateway

original = gateway.identity_private_allowed


def allowed(method, path):
    return (method == "GET" and path == "/internal/identity/metrics") or original(method, path)


gateway.identity_private_allowed = allowed

if __name__ == "__main__":
    gateway.main()
