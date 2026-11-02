#!/usr/bin/env python3
"""Thin canonical offline recovery CLI. Connections and keys use private files."""
import argparse
import json
import sys

import core


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("init", "capture", "capture-deny", "restore-new",
                                               "reconcile", "rearm", "gate", "status"))
    parser.add_argument("--config", required=True, help="Absolute owner-only mode-0600 JSON configuration")
    parser.add_argument("--minimum-sequence", type=int,
                        help="Independent minimum sequence retained outside the restore set")
    parser.add_argument("--rearm-file", help="Absolute mode-0600 explicit fixture rearm inputs; never raw secrets in argv")
    args = parser.parse_args()
    try:
        config = core.load_config(args.config)
        if args.operation == "init":
            result = core.initialize(config)
        else:
            material = core.keys(config)
            if args.operation == "capture":
                result = core.capture(config, material)
            else:
                core.need(args.minimum_sequence is not None and args.minimum_sequence >= 1,
                          "EXTERNAL_MINIMUM_SEQUENCE_REQUIRED")
                if args.operation == "rearm":
                    core.need(args.rearm_file is not None, "REARM_FILE_REQUIRED")
                    result = core.rearm(config, material, args.minimum_sequence, args.rearm_file)
                else:
                    operations = {"capture-deny": core.capture_deny, "restore-new": core.restore_new,
                                  "reconcile": core.reconcile, "gate": core.gate, "status": core.status}
                    result = operations[args.operation](config, material, args.minimum_sequence)
        print(json.dumps(result, sort_keys=True))
        return 0
    except Exception as error:
        # Never stringify a DB/tool/JSON/crypto exception or traceback: it can
        # include raw SQL, credentials, a PHC, ciphertext or caller-controlled text.
        category = str(error) if isinstance(error, core.RecoveryError) else "RECOVERY_REJECTED"
        if not category.startswith("RECOVERY_") or not category.replace("_", "").isalnum():
            category = "RECOVERY_REJECTED"
        print(json.dumps({"status": "QUARANTINED", "code": category}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
