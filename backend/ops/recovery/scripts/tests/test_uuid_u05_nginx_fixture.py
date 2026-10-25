"""Explicit disposable network-none fixture; no host ports or business mounts."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("u05_runtime", Path(__file__).parents[1] / "prepare-uuid-u05-runtime.py")
RUNTIME = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNTIME)

PROBE = r'''
import http.server,threading,json,urllib.request,urllib.error,time
class Fake(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=json.dumps({'path':self.path,'authorization':self.headers.get('Authorization'),'cookie':self.headers.get('Cookie'),'upgrade':self.headers.get('Upgrade')}).encode()
  self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(body)
 def log_message(self,*args):pass
for port in (13001,13002,13007,13010,13011):
 server=http.server.HTTPServer(('127.0.0.1',port),Fake);threading.Thread(target=server.serve_forever,daemon=True).start()
def get(path,headers=None):
 request=urllib.request.Request('http://127.0.0.1:13000'+path,headers=headers or {})
 try:
  with urllib.request.urlopen(request,timeout=3) as response:return response.status,response.read(),dict(response.headers)
 except urllib.error.HTTPError as error:return error.code,error.read(),dict(error.headers)
for _ in range(30):
 try:
  assert get('/healthz')[0]==200;break
 except Exception:time.sleep(.1)
else:raise RuntimeError('isolated nginx not ready')
checks=0
for path in ('/','/sub/vue/','/sub/jquery/index.html','/sub-apps/stores/versions/1.0.1-uuid-u05/','/callback','/micro/work-orders/tickets/synthetic'):
 assert get(path,{'Accept':'text/html'})[0]==200,path;checks+=1
for path in ('/missing.js','/sub/vue/missing.js','/sub-apps/stores/versions/old/index.html','/stores-api/internal/check','/stores-api/%69nternal/check','/oauth-playground-api/metrics','/sub/unknown/'):
 assert get(path,{'Accept':'text/html'})[0]==404,path;checks+=1
for prefix in ('stores-api','work-orders-api','api','oauth','micro-proxy'):
 status,body,_=get('/'+prefix+'/fixture?x=1');assert status==200 and json.loads(body)['path']=='/'+prefix+'/fixture?x=1';checks+=1
status,body,_=get('/oauth-playground-api/flows',{'Authorization':'Bearer synthetic-platform-secret','Cookie':'platform_session=secret; oauth_playground=synthetic-oauth; other=private'})
value=json.loads(body);assert status==200 and value['path']=='/api/flows' and value['authorization'] is None and value['cookie']=='oauth_playground=synthetic-oauth';checks+=1
for extension in ('js','mjs'):
 assert 'javascript' in get('/sub/vue/app.'+extension)[2]['Content-Type'];checks+=1
status,body,_=get('/socket.io/?transport=websocket',{'Upgrade':'websocket','Connection':'Upgrade'})
assert status==200 and json.loads(body)['upgrade']=='websocket';checks+=1
print(json.dumps({'status':'PASS','checks':checks,'business_services_accessed':0}))
'''


@unittest.skipUnless(os.environ.get("UUID_U05_NGINX_FIXTURE") == "isolated-network-none-v1", "explicit isolated Docker fixture required")
class NginxFixture(unittest.TestCase):
    def test_actual_runner_exec_clears_inherited_environment_and_uses_private_logs(self):
        with tempfile.TemporaryDirectory(prefix="u05-runner-fixture-") as temporary:
            root = Path(temporary)
            state = root / "state"
            state.mkdir(mode=0o700)
            runner = root / "runner.py"
            runner.write_text(RUNTIME.RUNNER)
            runner.chmod(0o600)
            binary = root / "service"
            binary.write_text("#!/usr/bin/python3\nimport os,json\nprint(json.dumps({'pg_inherited':'PGPASSWORD' in os.environ,'jwt_is_fresh':os.environ.get('JWT_SECRET')=='synthetic-new-jwt','unrelated_inherited':'UNRELATED_CANARY' in os.environ,'policy_exact':os.environ.get('DATA_GRANT_POLICY')=='{\"applications\":[],\"managementScopes\":[]}'}))\n")
            binary.chmod(0o700)
            settings = root / "service.env"
            settings.write_text('JWT_SECRET=synthetic-new-jwt\nDATA_GRANT_POLICY={"applications":[],"managementScopes":[]}\n')
            settings.chmod(0o600)
            command = ["docker", "run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--env", "PGPASSWORD=synthetic-old-password", "--env", "JWT_SECRET=synthetic-old-jwt", "--env", "UNRELATED_CANARY=synthetic-old-state"]
            for path, name, readonly in ((runner, "runner.py", True), (binary, "service", True), (settings, "service.env", True), (state, "state", False)):
                command += ["--mount", f"type=bind,source={path},target=/run/u05/{name}" + (",readonly" if readonly else "")]
            command += ["--entrypoint", "/usr/bin/python3", "saas-ci:phase04-tools", "/run/u05/runner.py", hashlib.sha256(binary.read_bytes()).hexdigest()]
            result = subprocess.run(command, text=True, capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertEqual(json.loads((state / "stdout.log").read_text()), {"pg_inherited": False, "jwt_is_fresh": True, "unrelated_inherited": False, "policy_exact": True})
            self.assertEqual((state / "stderr.log").read_text(), "")
            self.assertEqual((state / "stdout.log").stat().st_mode & 0o777, 0o600)

    def test_actual_static_proxy_and_oauth_boundaries_in_private_network_namespace(self):
        token = secrets.token_hex(12)
        container = None
        with tempfile.TemporaryDirectory(prefix="u05-nginx-fixture-") as temporary:
            base = Path(temporary)
            static = base / "static"
            static.mkdir(mode=0o755)
            artifacts = {}
            for app in RUNTIME.APPS:
                directory = static / app
                directory.mkdir(mode=0o755)
                for filename, body in (("index.html", "<!doctype html><p>synthetic</p>"), ("app.js", "globalThis.fixture=true"), ("app.mjs", "export default true")):
                    (directory / filename).write_text(body)
                    (directory / filename).chmod(0o644)
                artifacts[app] = {"version": "1.0.1-uuid-u05"}
            (static / "oauth-playground/callback.html").write_text("<!doctype html><p>callback</p>")
            (static / "oauth-playground/callback.html").chmod(0o644)
            config = base / "nginx.conf"
            config.write_text(RUNTIME.nginx_configuration(artifacts))
            config.chmod(0o644)
            try:
                image = subprocess.check_output(["docker", "image", "inspect", "nginx:1.28.0-bookworm", "--format", "{{.Id}}"], text=True).strip()
                self.assertRegex(image, r"^sha256:[0-9a-f]{64}$")
                command = ["docker", "run", "-d", "--network", "none", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--user", "101:101", "--label", "saas.u05-fixture=" + token, "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=32m", "--mount", f"type=bind,source={static},target=/srv/frontends,readonly", "--mount", f"type=bind,source={config},target=/etc/nginx/nginx.conf,readonly", "--entrypoint", "/usr/sbin/nginx", image, "-g", "daemon off;"]
                container = subprocess.check_output(command, text=True).strip()
                self.assertRegex(container, r"^[0-9a-f]{64}$")
                probe = subprocess.run(["docker", "run", "--rm", "-i", "--network", "container:" + container, "--entrypoint", "python3", "saas-ci:phase04-tools", "-"], input=PROBE, text=True, capture_output=True, timeout=45)
                if probe.returncode:
                    logs = subprocess.run(["docker", "logs", container], text=True, capture_output=True, timeout=10)
                    self.fail("synthetic fixture failed: " + probe.stderr + logs.stderr)
                self.assertEqual(json.loads(probe.stdout), {"status": "PASS", "checks": 22, "business_services_accessed": 0})
            finally:
                if container and re.fullmatch(r"[0-9a-f]{64}", container):
                    state = json.loads(subprocess.check_output(["docker", "inspect", container], text=True))[0]
                    self.assertEqual(state["Id"], container)
                    self.assertEqual(state["Config"]["Labels"].get("saas.u05-fixture"), token)
                    self.assertEqual(state["HostConfig"]["NetworkMode"], "none")
                    self.assertFalse(state["HostConfig"]["PortBindings"])
                    subprocess.run(["docker", "rm", "-f", container], check=True, capture_output=True, timeout=15)


if __name__ == "__main__":
    unittest.main()
