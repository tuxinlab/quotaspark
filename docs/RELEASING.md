# 发版与签名流程

应用内置 Tauri 官方更新器（`tauri-plugin-updater`）。发新版固定五步：

## 0. 签名密钥

- 私钥：`~/.tauri/quotaspark.key`（公钥已写入 `src-tauri/tauri.conf.json`）
- **私钥丢失则无法再发布更新，务必备份**；私钥绝不入库（.gitignore 已排除 `*.key`）

## 1. 改版本号（三处同步）

`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`

## 2. 打包并签名

```bash
TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/quotaspark.key) \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
npm run tauri build
```

产物在 `src-tauri/target/release/bundle/`，安装包旁会生成同名 `.sig` 签名文件
（NSIS 的 `QuotaSpark_x.y.z_x64-setup.exe` 是发给用户的那个；`.msi` 是企业部署备用格式）。

## 3. 更新 latest.json

项目根目录的 `latest.json`：改 `version`、`notes`、`pub_date`，
`signature` 填 `.sig` 文件的完整内容，`url` 指向本次 Release 的安装包地址：

```json
{
  "version": "0.2.0",
  "notes": "更新说明",
  "pub_date": "2026-09-29T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "（.sig 文件的内容）",
      "url": "https://github.com/tuxinlab/quotaspark/releases/download/v0.2.0/QuotaSpark_0.2.0_x64-setup.exe"
    }
  }
}
```

## 4. 提交推送代码

```bash
git add -A && git commit -m "release: v0.2.0" && git push
```

## 5. GitHub Release

仓库页 → Releases → Draft a new release → 新建 tag `v0.2.0` → 上传三件套
（安装包 + `.sig` + `latest.json`，附件本身的 label 留空）→
**Release label 选 Latest** → Publish。

注意：新 UI 的 Release label 是发布级选项（None / Pre-release / Latest），
必须选 Latest——更新器端点是 `releases/latest/download/latest.json`，
选 None 会让所有用户收不到新版本。

## 6. 用户端的更新流程

应用启动时静默检查一次，之后每 6 小时复查一次，顶栏按钮也可手动检查。发现新版本
只把按钮变成「下载并安装 vX」并提示一次，不会自动安装。

用户点「下载并安装」后：下载安装包，用 `tauri.conf.json` 里的公钥验签，启动 NSIS
安装器，应用随即退出；安装器关闭残留进程、覆盖安装，并自动重新打开新版本。
没有单独的「重启」步骤。

维护者需要知道的几点：

- 签名里记录了被签安装包的版本号（本项目 0.1.0 起的签名都带），更新器会校验它与
  `latest.json` 宣告的 `version` 一致，不一致就拒绝安装。所以 `latest.json` 的
  `version` 必须和上传的安装包是同一个版本，不能只改数字；打包也请始终用
  `npm run tauri build`，不要用别的工具代签。
- 参与比较的「当前版本」取自 `tauri.conf.json` 的 `version`，发版时它必须和
  `latest.json` 的 `version` 一起改。
- 更新端点在 `src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints`，仓库归属是
  `tuxinlab`。旧组织名 `tuxin-labs` 目前靠 GitHub 的 301 重定向还能访问，但不要再
  写回旧名字；0.1.7 及更早的版本编译进去的是旧名字，只能靠这个重定向。
