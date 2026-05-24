# M41.8-A BI 远程元数据接口技术记录

## 数据来源

- BI 前端代码库: `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/`
- 核心文件: `metadata/metadata.ts`、`types/metadata-meta-types.d.ts`、`sys/sys.ts`
- metadata-checker 已有模块: `src/session/remote_provider.rs`、`src/remote_metadata.rs`、`src/remote_metadata_provider.rs`

## 认证层

BI 前端通过 `ctx(url)` 自动添加 context path，通过 cookie/session 维持登录态。
关键认证接口：

- `/api/me/getPermissionInfo` —— 返回当前用户权限信息（含项目列表）
- 数据经过 `LZString.decompressFromBase64()` 解压，原始载荷为 Base64 编码的 JSON

## 接口清单

### 1. 项目列表接口

**请求**

```
GET /api/me/getPermissionInfo
Content-Type: application/json
```

- 前端调用: `rc1("/api/me/getPermissionInfo")`
- 数据格式: 返回字符串，需 `JSON.parse(LZString.decompressFromBase64(data))`

**响应字段（解压后）**

```typescript
interface UserPermissionInfo {
    isAdmin: boolean;
    anonymous: boolean;
    userDirectory: string;      // "sys" | "external"
    loginType: LoginType;
    szTypes: string[];
    protectedFiles: string[];
    protectedProjectFiles: string[];
    userId: string;
    userName: string;
    loginTime: number;
    userInfo: UserInfo;
    groups?: UserGroupInfo[];
    permissions?: { [operation: string]: PermissionTrie };
    metaProjects: MetaProjectInfo[];      // <-- 项目列表
    userFieldsInfo: { [fieldName: string]: { desc?: string; isDimension?: boolean; fieldsInfo: {...} } };
}
```

**MetaProjectInfo 字段**

```typescript
interface MetaProjectInfo {
    projectName: string;    // 项目标识（如 xiaoshouyi、sysdata）
    desc?: string;          // 项目描述
    icon?: string;          // 项目图标
    // 继承 MetaFileInfo 的字段
    id: UUID;
    path: ResourcePath;     // 如 /xiaoshouyi
    parentDir: string;
    name: string;
    type?: string;
    revision?: string;
    isFolder?: boolean;
    createTime?: number;
    modifyTime?: number;
    creator?: string;
    modifier?: string;
    // ... 其他 MetaFileInfo 字段
}
```

### 2. 文件子列表接口

**请求**

```
GET /api/meta/services/getFileChildren/{projectName}
```

- 前端调用: `rc1("/api/meta/services/getFileChildren/" + encodeURIComponent(projectName))`
- 示例: `/api/meta/services/getFileChildren/xiaoshouyi`

**响应字段**

```typescript
{
    file: MetaProjectInfo,          // 项目根信息
    children: MetaFileInfo[],       // 直接子文件/文件夹列表
    users: UserInfo[]              // 相关用户信息（可选）
}
```

### 3. 文件后代接口（全量树）

**请求**

```
GET /api/meta/services/getFileDescendant/{path}
```

- 前端调用: `rc1("/api/meta/services/getFileDescendant/" + encodeURI(path.substring(1)))`
- path 为项目内路径，如 `xiaoshouyi/data/tables`
- 示例: `/api/meta/services/getFileDescendant/xiaoshouyi/data/tables`
- 缓存策略: 后端使用 etag 协商缓存

**响应字段**

返回 `MetaFileInfo[]` 扁平数组，包含所有后代文件信息。前端通过 `parentDir` 字段重建树结构。

### 4. 文件信息接口

**请求**

```
GET /api/meta/services/getFileInfo/{fileIdOrPath}
```

- 前端调用: `makeMetaFileUrl("/api/meta/services/getFileInfo/", fileInfoOrPathOrId)`
- URL 构造规则:
  - 若传入 `MetaFileInfo` 对象，取 `file.id` 作为 URL 路径段
  - 若传入路径字符串，需 `encodeURIComponent` 转码
- 示例: `/api/meta/services/getFileInfo/abc123UUID`

**响应字段**

```typescript
interface MetaFileInfo {
    projectName?: string;       // 所属项目
    id: UUID;                   // 文件唯一 ID
    path: ResourcePath;         // 完整路径，如 /xiaoshouyi/data/tables/销售/fact_saleContract.tbl
    parentDir: string;          // 父目录路径
    type?: string;              // 文件类型: spg, tbl, dash, app, fold 等
    name: string;               // 文件名（含扩展名）
    desc?: string;              // 描述
    revision?: string;          // 版本号
    isFolder?: boolean;         // 是否为文件夹
    icon?: string;              // 图标
    createTime?: number;        // 创建时间（毫秒时间戳）
    creator?: string;           // 创建者 ID
    creatorName?: string;       // 创建者名称
    modifyTime?: number;        // 修改时间（毫秒时间戳）
    modifier?: string;          // 修改者 ID
    modifierName?: string;      // 修改者名称
    order?: number;             // 排序字段
    content?: string;           // 文件内容（文本）
    contentJSON?: JSONObject;   // JSON 解析后的内容
    children?: MetaFileInfo[];  // 子文件列表（文件夹时）
    linkages?: MetaFileInfo[];  // 血统引用
    impacts?: MetaFileInfo[];   // 影响引用
    bussinessObject?: any;      // 业务对象
    allFilesLoaded?: boolean;   // 是否已加载所有子文件
    thumbnail?: string;         // 缩略图地址
    subFileType?: string;       // 子类型（如 tbl 下分 app, ods, sql, dataflow）
}
```

### 5. 文件内容接口

**请求**

```
GET /api/meta/services/getFileContent/{fileId}
```

- 前端调用: `makeMetaFileUrl("/api/meta/services/getFileContent/", finf)`
- 支持查询参数:
  - `v={etag_version}` —— 用于 HTTP 强缓存（取自 `getFileEtagVersion(file)`）
  - `revision={revision}` —— 请求特定版本内容
- dataType: "text"（前端明确指定只接收文本）
- 示例: `/api/meta/services/getFileContent/abc123UUID?v=202108131814-7d8157471172`

**响应**

返回文件原始内容字符串（JSON 文本、XML 文本或其他）。

### 6. 其他相关接口

| 接口 | 路径 | 用途 |
|------|------|------|
| 编译后内容 | `/api/meta/services/getCompiledInfoContent/{fileId}` | 获取编译后的元数据内容 |
| 主题信息 | `/api/meta/services/getThemeInfo/{project}/{type}/{name}.json` | 获取主题配置 |
| 缩略图 | `/api/meta/services/getFileThumbnail/{fileId}?size={size}&v={v}` | 获取文件缩略图 |
| 附件 | `/api/meta/services/attachment` | 附件上传/下载 |
| 影响分析 | `/api/meta/services/impactAnalysis` | 元数据影响分析 |

## 字段映射到 RemoteSessionProvider

| BI 接口 | BI 字段 | RemoteSessionProvider 字段 | 说明 |
|---------|---------|---------------------------|------|
| `getPermissionInfo.metaProjects[]` | `projectName` | `RemoteProjectInfo.project_ref` | 项目唯一标识 |
| `getPermissionInfo.metaProjects[]` | `desc` | `RemoteProjectInfo.project_name` | 项目展示名 |
| `getPermissionInfo.metaProjects[]` | `path` | `RemoteProjectInfo.source_origin` | 项目源路径 |
| `getFileChildren/getFileDescendant` | `id` | `RemoteMetafileEntry.file_id` | 文件 ID |
| `getFileChildren/getFileDescendant` | `path` | `RemoteMetafileEntry.source_path` | 项目内逻辑路径 |
| `getFileChildren/getFileDescendant` | `revision` | `RemoteMetafileEntry.revision` | 版本号 |
| `getFileChildren/getFileDescendant` | `modifyTime` | `RemoteMetafileEntry.mtime` | 修改时间（毫秒） |
| `getFileChildren/getFileDescendant` | — | `RemoteMetafileEntry.etag` | 从 `v` 参数或响应头推导 |
| `getFileChildren/getFileDescendant` | — | `RemoteMetafileEntry.size` | 需从 content 长度推导或额外获取 |
| `getFileChildren/getFileDescendant` | `isFolder` | `RemoteMetafileEntry.deleted` | 文件夹不删除，文件按状态标记 |
| `getFileContent` | 响应体 | `RemoteFileContent.raw_text` | 原始文本内容 |
| `getFileContent` | `type` | `RemoteFileInfo.content_type` | 从扩展名推断 |

## 关键约束

1. **数据压缩**: `getPermissionInfo` 返回的数据需先 Base64 解码，再用 `LZString.decompressFromBase64` 解压，最后 JSON 解析。
2. **路径编码**: 元数据名称可包含特殊字符，URL 路径段需 `encodeURIComponent` 编码（`encodeURI` 不足以处理所有特殊字符）。
3. **缓存策略**: `getFileDescendant` 和 `getFileContent` 使用 etag 协商缓存，通过 `v` 查询参数实现 HTTP 强缓存。
4. **认证依赖**: 所有接口依赖当前登录 session（cookie），无显式 token 传递。
5. **内容类型**: `getFileContent` 需显式指定 `dataType: "text"`，避免后端返回对象。

## 与已有 RemoteMetadataProvider 的差异

| 维度 | `RemoteMetadataProvider` (M40.6/M41.2) | BI 实际接口 |
|------|----------------------------------------|-------------|
| 项目列表 | `list_projects()` 直接返回 | 从 `getPermissionInfo` 中提取 |
| 文件列表 | `list_metafiles()` 直接返回 | 需先 `getFileChildren` 再 `getFileDescendant` |
| 认证 | trait 层面未定义认证 | 依赖 cookie/session，无显式 token |
| 数据压缩 | 无压缩 | `getPermissionInfo` 使用 LZString+Base64 |
| 缓存标记 | `etag` 字段 | 实际通过 `v` 查询参数和 HTTP etag 头 |
| 版本 | `revision` 字段 | BI 中 `revision` 为字符串 |

## 下一步实现建议

1. **M41.8-B**: 实现 `RemoteSessionProvider` 的 reqwest 版本时，需处理:
   - Cookie jar 管理（自动维持 session）
   - `getPermissionInfo` 的 LZString+Base64 解压
   - 路径的 `encodeURIComponent` 编码
   - 404/401/403 错误码映射到 `RemoteMetadataErrorCode`

2. **M41.10-A**: `sync_remote_files_to_session` 时:
   - 先调用 `list_projects()` 获取项目列表
   - 对每个项目调用 `list_metafiles()` 获取文件树
   - 对变化的文件调用 `fetch_metafile_content()` 写入 session mirror
   - 更新 `SessionManifest`

3. **M41.11**: 增量同步时:
   - 使用 `revision` 或 `mtime` 判断文件是否变化
   - 删除的文件在 manifest 中标记 `deleted=true`
   - 从 session mirror 中物理移除
