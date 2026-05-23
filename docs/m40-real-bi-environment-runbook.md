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

## 7. 快速更新远程胶水 JS

项目级前端 hook 的真实加载路径是：

```text
/{projectName}/public/hooks/custom.js
```

例如 `analyzer` 项目：

```text
/analyzer/public/hooks/custom.js
```

`custom.ts` 是源码文件；平台编辑器保存时会在前端 Monaco 中编译并同步保存 `custom.js`。如果已经生成了可直接运行的 AMD 格式胶水 JS，可以直接更新 `custom.js`。

仓库提供了测试工具：

```bash
node browser/tools/remote-metadata-uploader.mjs \
  --base-url 'https://autocrm-test.xiaoshouyi.com' \
  --login-body-file /private/tmp/autocrm-login-payload.json \
  --file /path/to/generated-glue.js \
  --remote-path /analyzer/public/hooks/custom.js
```

也可以使用环境变量传登录信息：

```bash
export MC_REMOTE_BASE_URL='https://autocrm-test.xiaoshouyi.com'
export MC_REMOTE_USERNAME='<username>'
export MC_REMOTE_PASSWORD='<password>'

node browser/tools/remote-metadata-uploader.mjs \
  --file /path/to/generated-glue.js \
  --remote-path /analyzer/public/hooks/custom.js
```

工具行为：

- 登录 `/api/auth/signin`。
- 自动确保父目录存在，例如 `/analyzer/public/hooks`。
- 文件不存在时调用 `/api/meta/file/createFile`。
- 文件存在时调用 `/api/meta/file/modifyFile`，并携带当前 `revision`。
- 保存后重新读取 `downloadContent=true`，确认远程内容和本地文件一致。
- 不会把密码或 cookie 写入输出。

本地测试命令：

```bash
node --test browser/test/remote-metadata-uploader-smoke.test.mjs
```

## 8. 失败排查

| 现象 | 优先检查 |
|---|---|
| `/api/auth/signin` 返回失败 | `cipherPassport` 是否是 base64 后的 JSON；`userDirectory` 是否正确；是否需要验证码 |
| 没有 `JSESSIONID` | 是否使用 `-c` 保存 cookie；是否被重定向到其它域 |
| 页面返回 401 | 是否使用 `-b "$AUTOBI_COOKIE_JAR"`；cookie 是否过期 |
| 页面 HTML 无 `customJSES` | 是否访问的是元数据页面而非登录页；是否带 `?:edit=true`；目标文件是否存在 |
| 只看到系统级脚本 | 当前项目或应用可能没有 `custom.js` 文件 |
| `onInitDesigner` 不执行 | 脚本是否正确导出；当前打开的是否是设计器；`custom.js` 是否被合并规则命中 |

## 9. SuperPage Designer hook 真实验证方法论

本节记录真实环境中验证 `onInitDesigner` 的推荐流程。不要只验证文件上传成功；必须证明浏览器端设计器加载了脚本，并且 hook 被调用。

### 9.1 先创建真实 app 容器

SuperPage 设计器不要用孤立路径 `/analyzer/app/Page.spg` 验证。真实应用页面通常位于 `.app` 目录下：

```text
/analyzer/app/M40HookSmoke.app/settings.json
/analyzer/app/M40HookSmoke.app/M40HookDesign.spg
```

最小 `settings.json`：

```json
{
  "version": "4.15.0",
  "shortUrls": null
}
```

上传示例：

```bash
node browser/tools/remote-metadata-uploader.mjs \
  --base-url 'https://autocrm-test.xiaoshouyi.com' \
  --login-body-file /private/tmp/autocrm-login-payload.json \
  --file /private/tmp/m40-hook-smoke-settings.json \
  --remote-path /analyzer/app/M40HookSmoke.app/settings.json

node browser/tools/remote-metadata-uploader.mjs \
  --base-url 'https://autocrm-test.xiaoshouyi.com' \
  --login-body-file /private/tmp/autocrm-login-payload.json \
  --file /path/to/real-superpage.spg \
  --remote-path /analyzer/app/M40HookSmoke.app/M40HookDesign.spg
```

注意：过小的手写 fixture 可能在真实设计器中报类似 `Cannot read properties of undefined (reading 'dimensions')` 的错误，导致设计器初始化没有走到 hook。验证 hook 时优先使用真实项目中能打开的 SuperPage 元数据样本。

### 9.2 用项目级 custom.js 做隔离验证

测试项目 `analyzer` 的项目级 hook 路径：

```text
/analyzer/public/hooks/custom.js
```

推荐先上传一个最小、可人工观察、也可自动化验收的 AMD 脚本：

```js
(function () {
  function mark(key, value) {
    var root = document.documentElement || document.body;
    if (root) {
      root.setAttribute(key, value);
    }
  }
  mark("data-metadata-checker-script-evaluated", "top-level");
  mark("data-metadata-checker-script-has-define", String(typeof define));
})();

define(["require", "exports"], function (require, exports) {
  "use strict";
  Object.defineProperty(exports, "__esModule", { value: true });

  var MARKER = "metadata-checker-m40-hook-smoke";

  function mark(key, value) {
    var root = document.documentElement || document.body;
    if (root) {
      root.setAttribute(key, value);
    }
  }

  mark("data-metadata-checker-module-loaded", MARKER);

  function onInitDesigner(designer, args) {
    console.log("[metadata-checker] onInitDesigner loaded", {
      href: location.href,
      designer: designer,
      args: args,
      at: new Date().toISOString()
    });
    mark("data-metadata-checker-on-init-designer", MARKER);
    mark("data-metadata-checker-on-init-href", location.href);
    mark("data-metadata-checker-on-init-path", args && (args.path || (args.file && args.file.path)) || "");
  }

  exports.onInitDesigner = onInitDesigner;
  exports.CustomJS = {
    "*": { onInitDesigner: onInitDesigner },
    "spg": { onInitDesigner: onInitDesigner },
    "M40HookDesign.spg": { onInitDesigner: onInitDesigner },
    "/analyzer/app/M40HookSmoke.app/M40HookDesign.spg": { onInitDesigner: onInitDesigner }
  };
});
```

其中：

- `console.log(...)` 用于人工打开浏览器控制台观察。
- `data-metadata-checker-script-evaluated` 证明脚本文件被浏览器执行。
- `data-metadata-checker-module-loaded` 证明 AMD factory 被执行。
- `data-metadata-checker-on-init-designer` 证明 `onInitDesigner` 被调用。
- `exports.onInitDesigner` 和 `exports.CustomJS["*"]` 同时保留，用于排除导出形态或匹配 key 差异。

### 9.3 打开设计器并确认注入链路

打开：

```text
https://autocrm-test.xiaoshouyi.com/analyzer/app/M40HookSmoke.app/M40HookDesign.spg?:edit=true
```

页面可能会规范化到：

```text
https://autocrm-test.xiaoshouyi.com/analyzer/app/M40HookSmoke.app?:edit=true
```

这是 app 设计器路由的正常行为，不代表运行在 iframe 中，也不代表目标页面丢失。

用 HTML 验证注入版本：

```bash
curl --compressed -sS -D /private/tmp/m40-hook-design.headers \
  -o /private/tmp/m40-hook-design.html \
  -b /private/tmp/autocrm-cookies.txt \
  'https://autocrm-test.xiaoshouyi.com/analyzer/app/M40HookSmoke.app/M40HookDesign.spg?:edit=true'

rg -n "customJSES|custom\\.js|M40HookSmoke|M40HookDesign" /private/tmp/m40-hook-design.html
```

期望看到：

```text
/sysdata/public/hooks/custom.js
/analyzer/public/hooks/custom.js
customJSES":{"sysdata":...,"analyzer":...,"/analyzer/app/M40HookSmoke.app":null}
```

这说明设计器 HTML 已经把项目级 hook 注入到 `sys.ready([...])`。

### 9.4 验收信号优先级

推荐按下面顺序判断：

1. 设计器 UI 正常打开，无页面级错误弹窗。
2. HTML 中存在 `/analyzer/public/hooks/custom.js?v=...`，且版本参数随上传时间更新。
3. 直接下载 `/analyzer/public/hooks/custom.js` 能看到最新脚本内容。
4. 浏览器控制台出现 `[metadata-checker] onInitDesigner loaded`。
5. DOM 上存在 `data-metadata-checker-on-init-designer`。

其中第 4 条适合人工验收，第 5 条适合自动化验收。不要只依赖 console，因为自动化工具未必稳定暴露控制台日志；也不要只依赖全局 `window.__xxx`，部分自动化执行上下文可能看不到页面脚本挂载的全局变量。

### 9.5 已知误区

- 不要把问题优先归因到 iframe。当前 SuperPage 设计器主流程不应运行在 iframe 中；只有页面内组件本身可能包含 iframe。
- 不要用过度简化的 `.spg` fixture 直接判断 hook 是否失败。fixture 可能先让设计器初始化报错。
- 不要只看远程上传工具返回 `modified`。必须继续查 HTML 注入、脚本响应和 hook 执行信号。
- 不要把 `custom.ts` 当成运行时入口。真实运行时入口是编译后的 `custom.js`。
