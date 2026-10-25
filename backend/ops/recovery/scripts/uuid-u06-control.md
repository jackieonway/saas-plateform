# U06 固定六库预创建与双代运行控制

`uuid-u06-control.py` 负责固定新 final/recovery 三库各一组、真实迁移与只读封存，以及单独调用的 U05 停写。`uuid-u06-switch.py` 负责已准备 U06 运行材料的核验、七服务互斥启动、捕获正在运行的代次和精确停写。两者都只能由本机 Linux root 执行；默认不自动启动、停止、恢复旧服务或开放角色。

每次操作必须使用对应固定 base 下全新的 root/0700 目录。意图、事件和回执为 root/0600，采用 create-new、fsync 和原始摘要；失败目录保留隔离，不能覆盖或自动重试。明文环境、凭据、业务行及原生数据库错误不输出到终端。

## 已完成的六库控制链

2026-10-24：`prepare-03 → create-01 → migrate-01 → seal-01` 成功。控制脚本 SHA-256 固定为 `269931966054c4492df7f33a93c5f4959900f019403e6ebc7da2cf755d63f45e`，cluster system identifier 为 `7694215878237645672`。回执目录 base 为 `/var/lib/saas-uuid-u06-control-20261024`。

| 回执 | 原始文件 SHA-256 |
|---|---|
| `prepare-03/receipt.json` | `ee16a0975958ce8713befbcb35debb86d0ba2e010dd41f2370eceb9de271557e` |
| `create-01/receipt.json` | `c6620a3bb6db2044f7d9cde43e0e89a6f3fe39f253e998f276bad59b06aa8e64` |
| `migrate-01/receipt.json` | `2ecb46fc50fe255c5912e5ad1463babf87d5d60fd3a6b0e75b66bd09e037518a` |
| `seal-01/receipt.json` | `1fd2b621e624c0e53a52ed742d02b9b7b2ff933d5b4e6d66d268e22962d11a3c` |

| set / kind | DB OID | owner OID |
|---|---:|---:|
| final / platform | 29247 | 29246 |
| final / stores | 29249 | 29248 |
| final / work_orders | 29251 | 29250 |
| recovery / platform | 29253 | 29252 |
| recovery / stores | 29255 | 29254 |
| recovery / work_orders | 29257 | 29256 |

完整名称、每代 raw schema pin、semantic pin、注册 DTO、表计数和 owner/ACL 证据都在 sealed receipt 的六项 `targets` 内。完整 raw catalog 在同目录 `schema-<DB>.json`，三份源 catalog 在 `prepare-03/schema-source-<kind>.json`。旧 prepare-01/02 保留，不能混入最终链。

每组使用三个保护的 migrate-only 二进制，参数仅 `--import`，真实迁移连续版本为 12/3/4。未 seed、未复制或伪造迁移表；平台仅有正式迁移创建的 `platform_settings` singleton。seal 后六库默认只读、owner 安全且 NOLOGIN、无成员关系、无其他 LOGIN 的 CONNECT/owner SET/admin 路径，backends/prepared 均为 0。PUBLIC 的 database 和 schema 权限已移除，后续 runtime role 需要显式 schema USAGE。

semantic digest 共享 `uuid-u06-data.py` 的纯函数，并将函数源码摘要绑定在回执中。仅内部外键触发器 `RI_ConstraintTrigger_[ac]_<数字>` 的名称及 definition 内唯一同名规范化为 `[OID]`；其它完整 catalog 项保持，最后 canonical 排序。未知内部触发器拒绝。source/final/recovery 同 kind 必须语义一致；raw 指纹仍各代单独 pin。

迁移子进程采用独立 session，超时关闭整个 process group 并等待消失，日志始终 fsync，失败时只允许终止精确新 OID/name/owner 的 backend，检查归零后保留失败证据。控制回执严格要求六项、顺序与固定 set/kind/name/owner 相符，OID 为合法整数且唯一，禁止短 `zip` 跳过后续迁移。

## U05 首次停写

仅由主控在全部业务/工具独审就绪后执行：

```sh
python3 -B uuid-u06-control.py --freeze-u05 \
  --receipt /var/lib/saas-uuid-u06-control-20261024/seal-01/receipt.json \
  --output /var/lib/saas-uuid-u06-control-20261024/freeze-01
```

它重新核对 cluster/postmaster、三源 OID/owner/raw schema/history、四 runtime role OID/settings、全 LOGIN CONNECT/SET/admin 闭合，以及七个原 U05 容器的完整 ID、启动代次、restart count、配置摘要、实际进程摘要、固定二进制、env/runner/empty、精确挂载和网关静态内容。四份实际 role guard 已在 prepare-03 的 READ ONLY 事务中执行验证。

停写顺序为 gateway、OAuth、chat、orders、stores、notification、platform。之后只将四个精确 U05 role 设为 NOLOGIN，并排空三个精确当前 U05 v7 源库。旧 v4 源保持原围栏。全角色准入闭合、容器停止、源 idle/defaultRO、角色设置保持全部通过后才发布 `FROZEN_U05_V7_SOURCES`。该操作单独存在，六库预创建不会调用它。未发生该操作时，不得把六库已 seal 当成源已停写。

## U06 final/recovery 运行控制

代码检查不会访问 DB/Docker；材料检查读取已准备的固定本代文件和原始受保护二进制：

```sh
python3 -B uuid-u06-switch.py --check-code
python3 -B uuid-u06-switch.py --check --generation final
```

`--check` 要求 [runtime 准备器](uuid-u06-runtime.md) 已生成 `/var/lib/saas-uuid-u06-runtime-20261024/<generation>`，且 [epoch 工具](prepare-uuid-u06-epoch.py) 已完成 activate。六个 `services.env_file` 必须精确指向 `/var/lib/saas-uuid-u06-20261024/epoch-<generation>/<service>.json` 并对应 prepared 的逐文件 hash。新 env、receipt/epoch、固定 runner/empty、八前端全文件、Nginx、二进制都重新验证。saved launch-plan 仅与代码重新构造的七条固定命令比较，任何 JSON argv 都不会直接执行。

只有主控显式 enable 本代四 runtime role 后才能启动；switch 不自动 LOGIN。所有旧 U05/U06 容器必须停止、其他代角色必须 NOLOGIN、三旧 U05 源必须继续封存、本代数据库默认只读且只有合法 runtime 逐库写 override。本代容器名和 state 必须全新，七个共享 loopback 端口必须空闲。

```sh
python3 -B uuid-u06-switch.py --start --generation final \
  --sealed-receipt /var/lib/saas-uuid-u06-control-20261024/seal-01/receipt.json \
  --output /var/lib/saas-uuid-u06-switch-20261024/start-final-01
```

成功状态仅为 `STARTED_NOT_HEALTH_VERIFIED`，主控仍须完成真实登录、业务副作用和旧凭据拒绝验收。start 或 freeze 开始 mutation 后发生失败，统一将已知本代四角色 NOLOGIN，逐个停止本次精确创建或 capture 的 ID，逐库排空本代数据库。角色 guard 允许当前已 NOLOGIN，避免部分成功阻止后续清理；角色 fence、每 ID stop、每库 idle 和最终 closure 分别尝试并记录，即使某步失败也继续其它步骤。未知 timeout ID 先按固定名发现，再完整核对静态 policy/security/image/mount/entrypoint，不能只凭 labels 停止。全部闭合才记录 `QUARANTINED_NOLOGIN_IDLE`；任何清理失败记录 `FAILED_CLEANUP_INCOMPLETE`。始终保留容器/数据库和失败目录，不自动换回旧代。

业务探针完成并清理临时授权后，捕获精确七容器配置与进程代次，再进行单独停写：

```sh
python3 -B uuid-u06-switch.py --capture --generation final \
  --sealed-receipt /var/lib/saas-uuid-u06-control-20261024/seal-01/receipt.json \
  --output /var/lib/saas-uuid-u06-switch-20261024/capture-final-01

python3 -B uuid-u06-switch.py --freeze \
  --receipt /var/lib/saas-uuid-u06-switch-20261024/capture-final-01/receipt.json \
  --output /var/lib/saas-uuid-u06-switch-20261024/freeze-final-01
```

freeze 再次核对 capture 的 code/helper/material/cluster/role/container 代次，停相同七服务、四角色 NOLOGIN、精确 OID backend 归零、默认只读和完整 LOGIN CONNECT/SET/admin 闭合，然后输出 `FROZEN_NOLOGIN_IDLE`、`downtime_started_at_utc` 和 freeze 耗时。该耗时是停写阶段记录，整体 RTO 仍由主控直到 recovery 业务验收完成计时。恢复数据与 fresh epoch 的准备、LOGIN 开放继续由相应工具负责；recovery 的启动命令将 `--generation` 改为 recovery，所有互斥与固定路径规则相同。

本工具同时持有控制 advisory lock 和 epoch admission advisory lock，防止同一工具族互相交叉变更。它们不能阻止受信任 root/超级用户有意绕过围栏。任何失败都不发布成功回执，不自动 DROP/删除、清空数据或恢复旧服务。

## 回归验证

```sh
python3 -B -m unittest discover -s tests -p test_uuid_u06_control.py -v
python3 -B -m unittest discover -s tests -p test_uuid_u06_switch.py -v
```

以上命令在 WSL Ubuntu Linux root 环境执行。control 的 7 项测试覆盖真实 fork 父子进程组 kill/reap、超时 finally fsync/backend0、receipt 截断/乱序/伪 OID 和全 LOGIN escalation。switch 的 12 项离线测试覆盖双代固定计划、拒绝任意 argv/image/root/二进制、七服务精确 policy/挂载/entrypoint、其它代运行及名称占用互斥、helper 代码漂移、freeze 首次 stop 失败触发完整 quarantine、role fence/单 ID stop/单库 idle 失败不跳过其余项、幂等 NOLOGIN 和 timeout 未知 ID 拒绝。switch 回归没有操作真实数据库、Docker、端口或服务；真实运行与两次停写须另行生成主控证据。
