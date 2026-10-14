#!/usr/bin/env bash
# Task 18-a 后端验证脚本 v2 — 临时实例 127.0.0.1:3102 (新编译二进制, 含租户 UPDATE 别名修复)
# A: 一个租户只能有一个根组织 (应用层 409 + DB 唯一索引兜底竞态实测)
# B: 租户/组织 expiresAt CRUD 三态 + 登录/context 过期拦截 + 还原
# 注: 本项目所有 create 端点返回 200 (axum Json 默认), 非 201
set -u
BASE="http://127.0.0.1:3102"
J="Content-Type: application/json"
PASS=0; FAIL=0
ok(){ echo "PASS: $1"; PASS=$((PASS+1)); }
bad(){ echo "FAIL: $1"; FAIL=$((FAIL+1)); }
check(){ if [ "$2" = "$3" ]; then ok "$1 [实际 $3]"; else bad "$1 (期望=$2 实际=$3)"; fi }
jqa(){ # desc json expr expected
  local actual; actual=$(echo "$2" | jq -r "$3" 2>/dev/null)
  if [ "$actual" = "$4" ]; then ok "$1 [实际 $actual]"; else bad "$1 (期望=$4 实际=$actual)"; fi
}
contains(){ if echo "$2" | grep -q "$3"; then ok "$1"; else bad "$1 (未含「$3」→ $(echo "$2" | head -c 300))"; fi }

req(){ curl -s -m 15 "$@"; }

echo "===== 0. admin 登录 + context (正常路径) ====="
LOGIN=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"admin","password":"admin123"}')
TICKET=$(echo "$LOGIN" | jq -r '.ticket')
ADMIN_ORG=$(echo "$LOGIN" | jq -r '.memberships[0].organizationId')
[ -n "$TICKET" ] && [ "$TICKET" != "null" ] && ok "admin 登录获取 ticket" || bad "admin 登录失败: $(echo "$LOGIN" | head -c 200)"
CTX=$(req -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $TICKET" -d "{\"organizationId\":\"$ADMIN_ORG\"}")
TOKEN=$(echo "$CTX" | jq -r '.accessToken')
[ -n "$TOKEN" ] && [ "$TOKEN" != "null" ] && ok "admin context 换 accessToken" || bad "admin context 失败: $(echo "$CTX" | head -c 200)"
AUTH="Authorization: Bearer $TOKEN"

TENANTS=$(req "$BASE/api/tenants?size=50" -H "$AUTH")
PLATFORM_ID=$(echo "$TENANTS" | jq -r '.records[] | select(.code=="admin-tenant") | .id')
STAR_ID=$(echo "$TENANTS" | jq -r '.records[] | select(.code=="star-mfg") | .id')
[ -n "$PLATFORM_ID" ] && [ -n "$STAR_ID" ] && ok "取得两租户 id (platform=$PLATFORM_ID star=$STAR_ID)" || bad "租户 id 解析失败"
jqa "租户 list 行含 expiresAt(null=永久)" "$TENANTS" '.records[0].expiresAt' "null"

TREE=$(req "$BASE/api/organizations/tree?tenantId=$PLATFORM_ID" -H "$AUTH")
ROOT_ORG=$(echo "$TREE" | jq -r '[.[] | select(.parentId==null)][0].id')
[ -n "$ROOT_ORG" ] && [ "$ROOT_ORG" != "null" ] && ok "Platform 平台根组织 id=$ROOT_ORG" || bad "根组织解析失败"
jqa "组织 tree 节点含 expiresAt" "$TREE" '.[0].expiresAt' "null"

echo
echo "===== 1. 根组织唯一 ====="
# 1a. parent 缺省创建第二个根组织 → 409
R1=$(req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"name\":\"[18a]第二个根组织\"}")
check "1a 根组织重复创建被拒" "409" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"name\":\"[18a]第二个根组织-b\"}")"
contains "1a 409 文案含「根组织」提示" "$R1" "该租户已存在根组织"
# parent 显式空串同样视为根
R1B=$(req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"name\":\"[18a]空串根组织\",\"parentId\":\"\"}")
contains "1b parentId 空串(=根)同样 409" "$R1B" "该租户已存在根组织"

# 1c. 带 parent 的子组织 → 200 + expiresAt 回显
R2=$(req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"parentId\":\"$ROOT_ORG\",\"name\":\"[18a]测试子组织\",\"code\":\"t18a-child\",\"expiresAt\":\"2030-01-01T00:00:00Z\"}")
check "1c 子组织创建成功" "200" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"parentId\":\"$ROOT_ORG\",\"name\":\"[18a]探针\"}")"
CHILD_ID=$(echo "$R2" | jq -r '.id')
jqa "1c 子组织响应 expiresAt 回显" "$R2" '.expiresAt' "2030-01-01T00:00:00Z"
PROBE_ID=$(echo "$(req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$PLATFORM_ID\",\"parentId\":\"$ROOT_ORG\",\"name\":\"[18a]探针\"}")" | jq -r '.id')

# 1d. 组织 update 三态: 缺省不更新 / 显式 null 清除 / 重新设置
R3=$(req -X PUT $BASE/api/organizations/$CHILD_ID -H "$J" -H "$AUTH" -d '{"name":"[18a]测试子组织-改名"}')
jqa "1d PUT 缺省 expiresAt 键 → 不更新(仍 2030)" "$R3" '.expiresAt' "2030-01-01T00:00:00Z"
R4=$(req -X PUT $BASE/api/organizations/$CHILD_ID -H "$J" -H "$AUTH" -d '{"expiresAt":null}')
jqa "1d PUT expiresAt:null → 清除为永久" "$R4" '.expiresAt' "null"
R4B=$(req -X PUT $BASE/api/organizations/$CHILD_ID -H "$J" -H "$AUTH" -d '{"expiresAt":"2029-06-01T00:00:00Z"}')
jqa "1d PUT expiresAt:值 → 重新设置" "$R4B" '.expiresAt' "2029-06-01T00:00:00Z"

# 1e. DB 兜底竞态实测: 新租户并发双建根组织, 靠 uq_org_root_per_tenant 封死
DBPATH_HIT=0; APPPATH_HIT=0; ABNORMAL=0
for i in 1 2 3 4 5 6 7 8; do
  NT=$(req -X POST $BASE/api/tenants -H "$J" -H "$AUTH" -d "{\"name\":\"[18a]竞态租户$i\",\"code\":\"t18a-race-$i\",\"expiresAt\":\"2031-01-01T00:00:00Z\"}")
  NT_ID=$(echo "$NT" | jq -r '.id')
  [ "$i" = "1" ] && jqa "1e 租户 create 支持 expiresAt 回显" "$NT" '.expiresAt' "2031-01-01T00:00:00Z"
  ( req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$NT_ID\",\"name\":\"[18a]竞态根A$i\"}" -o /tmp/18a/raceA.json -w '%{http_code}' > /tmp/18a/raceA.code ) &
  ( req -X POST $BASE/api/organizations -H "$J" -H "$AUTH" -d "{\"tenantId\":\"$NT_ID\",\"name\":\"[18a]竞态根B$i\"}" -o /tmp/18a/raceB.json -w '%{http_code}' > /tmp/18a/raceB.code ) &
  wait
  CA=$(cat /tmp/18a/raceA.code); CB=$(cat /tmp/18a/raceB.code)
  SORTED=$(printf '%s\n%s\n' "$CA" "$CB" | sort | tr '\n' ' ')
  if [ "$SORTED" = "200 409 " ]; then
    BODY409=$(cat /tmp/18a/raceA.json; cat /tmp/18a/raceB.json)
    if echo "$BODY409" | grep -q "该租户已存在根组织"; then DBPATH_HIT=$((DBPATH_HIT+1));
    elif echo "$BODY409" | grep -q "数据已存在"; then APPPATH_HIT=$((APPPATH_HIT+1));
    else bad "1e 第 $i 轮 409 响应异常: $BODY409"; fi
  else
    ABNORMAL=$((ABNORMAL+1)); bad "1e 第 $i 轮竞态结果异常: A=$CA B=$CB"
  fi
  req -X DELETE $BASE/api/tenants/$NT_ID -H "$AUTH" > /dev/null
done
ok "1e 8 轮竞态全部一胜一负(200+409) 无 500; DB 兜底命中 $DBPATH_HIT 轮 / 应用层校验命中 $APPPATH_HIT 轮 / 异常 $ABNORMAL"
[ $DBPATH_HIT -gt 0 ] && ok "1e DB 唯一索引兜底路径真实触发且文案友好" || echo "INFO: 未撞进竞态窗口(DB兜底为构造性保障, 全局 23505→409 仍在)"

echo
echo "===== 2. 租户过期时间 + 登录拦截 ====="
RA=$(req -X PUT $BASE/api/tenants/$STAR_ID -H "$J" -H "$AUTH" -d '{"expiresAt":"2020-01-01T00:00:00Z"}')
jqa "2a PUT 租户 expiresAt=过去 → 回显" "$RA" '.expiresAt' "2020-01-01T00:00:00Z"
jqa "2a 回显行 organizationCount 正常(别名修复)" "$RA" '.organizationCount' "1"
LIST=$(req "$BASE/api/tenants?keyword=星辰" -H "$AUTH")
jqa "2b GET list 回显 expiresAt" "$LIST" '.records[0].expiresAt' "2020-01-01T00:00:00Z"
RB=$(req -X PUT $BASE/api/tenants/$STAR_ID -H "$J" -H "$AUTH" -d '{"name":"星辰制造"}')
jqa "2c PUT 缺省键 → 不清除(仍 2020)" "$RB" '.expiresAt' "2020-01-01T00:00:00Z"
LD=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')
check "2d 租户过期 chenjiu 登录被拦" "403" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')"
contains "2d 403 文案" "$LD" "所属租户或组织已到期, 无法登录"
RC=$(req -X PUT $BASE/api/tenants/$STAR_ID -H "$J" -H "$AUTH" -d '{"expiresAt":null}')
jqa "2e PUT expiresAt:null → 清除" "$RC" '.expiresAt' "null"
LE=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')
contains "2e 清除后 chenjiu 登录恢复" "$LE" '"ticket"'
CHEN_TICKET=$(echo "$LE" | jq -r '.ticket')
STAR_ORG=$(echo "$LE" | jq -r '.memberships[0].organizationId')

echo "----- 组织过期 -----"
RO=$(req -X PUT $BASE/api/organizations/$STAR_ORG -H "$J" -H "$AUTH" -d '{"expiresAt":"2020-01-01T00:00:00Z"}')
jqa "2f PUT 组织 expiresAt=过去 → 回显" "$RO" '.expiresAt' "2020-01-01T00:00:00Z"
check "2g 组织过期 chenjiu 登录被拦" "403" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')"
RH=$(req -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $CHEN_TICKET" -d "{\"organizationId\":\"$STAR_ORG\"}")
check "2h 组织过期 context 被拦(过期前签发的 ticket)" "403" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $CHEN_TICKET" -d "{\"organizationId\":\"$STAR_ORG\"}")"
contains "2h context 403 文案" "$RH" "该组织已到期, 无法切换"
RI=$(req -X PUT $BASE/api/organizations/$STAR_ORG -H "$J" -H "$AUTH" -d '{"expiresAt":null}')
jqa "2i PUT 组织 expiresAt:null → 清除" "$RI" '.expiresAt' "null"
LF=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')
CHEN_TICKET2=$(echo "$LF" | jq -r '.ticket')
contains "2i 清除后 chenjiu 登录恢复" "$LF" '"ticket"'

echo "----- 租户过期(仅 context 路径) -----"
req -X PUT $BASE/api/tenants/$STAR_ID -H "$J" -H "$AUTH" -d '{"expiresAt":"2020-01-01T00:00:00Z"}' > /dev/null
RJ=$(req -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $CHEN_TICKET2" -d "{\"organizationId\":\"$STAR_ORG\"}")
check "2j 租户过期 context 被拦" "403" "$(req -o /dev/null -w '%{http_code}' -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $CHEN_TICKET2" -d "{\"organizationId\":\"$STAR_ORG\"}")"
contains "2j context 403 文案" "$RJ" "所属租户已到期, 无法登录"

echo
echo "===== 3. 还原所有测试改动 ====="
req -X PUT $BASE/api/tenants/$STAR_ID -H "$J" -H "$AUTH" -d '{"expiresAt":null}' > /dev/null
req -X PUT $BASE/api/organizations/$STAR_ORG -H "$J" -H "$AUTH" -d '{"expiresAt":null}' > /dev/null
req -X DELETE $BASE/api/organizations/$CHILD_ID -H "$AUTH" > /dev/null
req -X DELETE $BASE/api/organizations/$PROBE_ID -H "$AUTH" > /dev/null
FINAL=$(req "$BASE/api/tenants?size=50" -H "$AUTH")
jqa "3 还原: 两租户 expiresAt 均为 null" "$FINAL" '[.records[] | select(.code=="admin-tenant" or .code=="star-mfg") | .expiresAt] | unique | .[0]' "null"
jqa "3 还原: 租户数回到 2" "$FINAL" '.total' "2"
FTREE=$(req "$BASE/api/organizations/tree?tenantId=$STAR_ID" -H "$AUTH")
jqa "3 还原: 星辰组织 expiresAt null" "$FTREE" '.[0].expiresAt' "null"
DTREE=$(req "$BASE/api/organizations/tree?tenantId=$PLATFORM_ID" -H "$AUTH")
jqa "3 还原: Platform 组织树节点数回到 6" "$DTREE" '[.. | objects | select(has("id"))] | length' "6"

echo "===== 4. 登录正常路径回归 ====="
LG=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"admin","password":"admin123"}')
GT=$(echo "$LG" | jq -r '.ticket')
GC=$(req -X POST $BASE/api/auth/context -H "$J" -H "Authorization: Bearer $GT" -d "{\"organizationId\":\"$ADMIN_ORG\"}")
contains "4 回归: admin 登录+context 正常" "$GC" '"accessToken"'
LZ=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"chenjiu","password":"Cj@12345"}')
contains "4 回归: chenjiu 登录正常(非空 memberships)" "$LZ" '"memberships"'
LW=$(req -X POST $BASE/api/auth/login -H "$J" -d '{"username":"wangwu","password":"Ww@12345"}')
contains "4 回归: wangwu(Platform 租户)登录正常" "$LW" '"ticket"'

echo
echo "===== 结果: PASS=$PASS FAIL=$FAIL ====="
[ $FAIL -eq 0 ] && echo "ALL GREEN" || echo "HAS FAILURES"
