# M40 真实 BI 环境测试流程

本文记录在真实 BI 环境中验证浏览器端插件接入点的可复现流程。目标是先从 BI 仓库源码确认登录、页面渲染和 `custom.js` 加载规则，再用真实服务器验证登录 cookie、页面 HTML 和用户自定义 JS 注入结果。

## 1. 源码定位

### 1.1 登录接口

前端登录逻辑位于 BI 仓库：

- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/security/login.ts`
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/metadata/metadata.ts`

当前登录请求形态：

- 方法：`POST`
- 路径：`/api/auth/signin`
- Content-Type：`application/json`
- 请求体字段：`cipherPassport`
- `cipherPassport` 内容：`base64(JSON.stringify(loginArgs))`

最小 `loginArgs`：

```json
{
  "user": "<username>",
  "password": "<password>",
  "remember": false,
  "userDirectory": "sys"
}
```

登录前可先读取：

- `GET /api/auth/options`

用于判断是否需要验证码、是否启用系统用户密码登录等。

### 1.2 用户自定义 JS 加载位置

BI 页面会加载三层用户自定义脚本：

| 作用域 | 元数据路径 | 示例 |
|---|---|---|
| 系统级 | `/sysdata/public/hooks/custom.js` | `/sysdata/public/hooks/custom.js` |
| 项目级 | `/{projectName}/public/hooks/custom.js` | `/xiaoshouyi/public/hooks/custom.js` |
| 应用级 | `{appPath}/custom.js` | `/xiaoshouyi/app/售后.app/custom.js` |

对应 CSS 路径同理：

- `/sysdata/public/hooks/custom.css`
- `/{projectName}/public/hooks/custom.css`
- `{appPath}/custom.css`

关键源码位置：

- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/src/main/java/com/succez/metadata/service/SysHookScriptsManager.java`
  - `SCRIPTFILE_CUSTOM_JS = "/public/hooks/custom.js"`
  - `SYS_SCRIPTFILE_CUSTOM_JS = "/sysdata/public/hooks/custom.js"`
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/src/main/java/com/succez/metadata/utils/MetaConst.java`
  - `CUSTOM_JS_SUFFIX = "/custom.js"`
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/src/main/java/com/succez/metadata/ActionMetaMgr.java`
  - 后端把系统级、项目级、应用级 `custom.js/custom.css` 拼入 `sys.ready([...])`
  - 同时把 `customJSES` 写入 `renderMetaFile(...)` 参数
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/metadata/metadata.ts`
  - `renderMetaFile(args)` 接收 `customJSES` 并写入 `allCustomJSMap`
  - `loadProjectCustomScript(project)` 加载项目脚本
  - `loadAppCustomScript(appPath)` 加载应用脚本
  - `getCustomJS(...)` 合并多层脚本
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/metadata/metadata-script-api.ts`
  - 定义 `onInitDesigner?(designer, args)`
- `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/dsn/dsnframe.ts`
  - 设计器初始化时调用 `customJS?.onInitDesigner?.(this, conf)`

## 2. 真实环境登录

不要把用户名、密码或 cookie 写入仓库。测试时使用环境变量和临时 cookie jar。

```bash
export AUTOBI_BASE='https://autocrm-test.xiaoshouyi.com'
export AUTOBI_USER='<username>'
export AUTOBI_PASSWORD='<password>'
export AUTOBI_COOKIE_JAR='/private/tmp/autocrm-cookies.txt'
```

### 2.1 查询登录选项

```bash
curl --compressed -sS \
  -c "$AUTOBI_COOKIE_JAR" \
  "$AUTOBI_BASE/api/auth/options"
```

检查点：

- HTTP 状态应为 `200`
- 若 `showCaptcha` 为 `true`，该流程需要补充验证码处理
- 若 `enableSysUserPasswordLogin` 为 `false`，不能使用系统用户密码登录

### 2.2 生成登录请求体

```bash
node -e '
const payload = {
  user: process.env.AUTOBI_USER,
  password: process.env.AUTOBI_PASSWORD,
  remember: false,
  userDirectory: "sys"
};
const body = {
  cipherPassport: Buffer.from(JSON.stringify(payload), "utf8").toString("base64")
};
process.stdout.write(JSON.stringify(body));
' > /private/tmp/autocrm-login-payload.json
```

### 2.3 登录并保存 cookie

```bash
curl --compressed -sS -D /private/tmp/autocrm-login-headers.txt \
  -o /private/tmp/autocrm-login-response.json \
  -b "$AUTOBI_COOKIE_JAR" \
  -c "$AUTOBI_COOKIE_JAR" \
  -H 'Content-Type: application/json' \
  --data-binary @/private/tmp/autocrm-login-payload.json \
  "$AUTOBI_BASE/api/auth/signin"
```

检查点：

- 响应 JSON 中 `result` 应为 `true`
- 响应 header 中应出现 `Set-Cookie: JSESSIONID=...`
- cookie jar 中应存在当前域名下的 `JSESSIONID`

## 3. 验证登录状态

使用一个需要登录态的接口验证 cookie 是否生效：

```bash
curl --compressed -sS -D /private/tmp/autocrm-me-headers.txt \
  -o /private/tmp/autocrm-me-response.txt \
  -b "$AUTOBI_COOKIE_JAR" \
  "$AUTOBI_BASE/api/me/getPermissionInfo"
```

检查点：

- HTTP 状态应为 `200`
- 不应返回登录页 HTML 或 `401`

## 4. 打开真实 SuperPage 设计器页面

使用一个真实页面验证后端渲染出的 HTML 是否包含 `custom.js` 注入信息。

```bash
export AUTOBI_SPG_PATH='/xiaoshouyi/app/售后.app/绑定车辆/会员已注册.spg'
export AUTOBI_SPG_URL="$AUTOBI_BASE$(node -e '
process.stdout.write(encodeURI(process.env.AUTOBI_SPG_PATH) + "?:edit=true");
')"

curl --compressed -sS -D /private/tmp/autocrm-designer-headers.txt \
  -o /private/tmp/autocrm-designer.html \
  -b "$AUTOBI_COOKIE_JAR" \
  "$AUTOBI_SPG_URL"
```

检查点：

- HTTP 状态应为 `200`
- HTML 中应包含 `renderMetaFile(...)`
- HTML 中应包含当前页面路径
- HTML 中应包含 `customJSES`

## 5. 验证 custom.js 注入结果

```bash
rg -n "customJSES|custom\\.js|public/hooks/custom" /private/tmp/autocrm-designer.html
```

对 `/xiaoshouyi/app/售后.app/绑定车辆/会员已注册.spg`，期望至少看到：

```text
/sysdata/public/hooks/custom.js
/xiaoshouyi/public/hooks/custom.js
/xiaoshouyi/app/售后.app/custom.js
customJSES":{"sysdata":...,"xiaoshouyi":...,"/xiaoshouyi/app/售后.app":...}
```

这说明当前页面已加载：

- 系统级 custom.js
- 项目级 custom.js
- 应用级 custom.js

其中 M40.5 的 SuperPage Designer glue 最适合通过应用级或测试专用项目级 `custom.js` 的 `onInitDesigner` 接入。

## 6. 后续新增元数据测试入口

拿到登录态后，新增元数据前必须先从 BI 仓库源码定位“新增/保存元数据”的真实接口和参数，不要猜接口。

建议顺序：

1. 在 BI 仓库中搜索新增文件相关调用：

   ```bash
   rg -n "createFile|newFile|saveFile|addFile|createMeta|saveMeta|getFile\\(" \
     /Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/metadata \
     /Users/wuhaocheng/Downloads/bi/com.succez.bi/src/main/java/com/succez/metadata
   ```

2. 确认前端封装函数、后端 action/service、请求体字段和权限要求。
3. 先在真实环境创建隔离测试目录或测试应用，避免污染生产业务应用。
4. 使用已保存的 cookie jar 调用新增接口。
5. 再用本流程重新打开页面，验证新增 `custom.js` 或测试页面是否被真实渲染链路加载。

## 7. 失败排查

| 现象 | 优先检查 |
|---|---|
| `/api/auth/signin` 返回失败 | `cipherPassport` 是否是 base64 后的 JSON；`userDirectory` 是否正确；是否需要验证码 |
| 没有 `JSESSIONID` | 是否使用 `-c` 保存 cookie；是否被重定向到其它域 |
| 页面返回 401 | 是否使用 `-b "$AUTOBI_COOKIE_JAR"`；cookie 是否过期 |
| 页面 HTML 无 `customJSES` | 是否访问的是元数据页面而非登录页；是否带 `?:edit=true`；目标文件是否存在 |
| 只看到系统级脚本 | 当前项目或应用可能没有 `custom.js` 文件 |
| `onInitDesigner` 不执行 | 脚本是否正确导出；当前打开的是否是设计器；`custom.js` 是否被合并规则命中 |
