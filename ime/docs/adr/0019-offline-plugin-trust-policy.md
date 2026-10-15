# ADR 0019：P3C 离线插件信任、强签名与时间戳边界

- 状态：P3C 已采用
- 日期：2026-08-02

## 背景

插件安装位于供应链边界。现有验证已要求 CMS SHA-256、代码签名 EKU、系统 Authenticode 信任链和缓存撤销状态，但只设置 `CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 并不能阻止链构建为 AIA 颁发者、CTL 或第三方根访问 URL。与此同时，P3C 没有冻结 RFC 3161/Authenticode 时间戳格式、时间戳机构策略和历史撤销语义，不能把未验证的签名时间当作过期证书的信任依据。

## 决策

1. P3C 正式安装使用完全离线、失败封闭的 Windows 信任策略。链只消费 CMS 内嵌证书、本机证书存储和已有缓存；同时设置 `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`、`CERT_CHAIN_DISABLE_AIA`、`CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY` 和 `CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT`。
2. 缺少中间证书、受信根、当前有效的缓存 CRL/OCSP，或撤销状态未知时一律拒绝。不忽略 `REVOCATION_STATUS_UNKNOWN`、`OFFLINE_REVOCATION`、过期、名称或用途错误。
3. CMS 分离签名与证书链都启用 Windows `szOID_CERT_STRONG_SIGN_OS_1`，避免随未来 SDK 的 `OS_CURRENT` 别名静默改变 v1 策略。该策略要求 SHA-2，RSA 密钥至少 2047 位或 ECDSA 至少 256 位，并对 CRL/OCSP 强签名进行检查；CMS SignerInfo 仍额外固定为 SHA-256。
4. 证书链按安装时当前系统时间验证。即使 CMS 携带 countersignature 或时间戳属性，P3C 也不把它用于过期证书降级，不输出“时间戳可信”状态。
5. 在线撤销刷新、RFC 3161/Authenticode 时间戳验证、历史签名有效性和发布者续签策略进入 P5“插件签名和发布流程”。未来开放必须使用显式新策略和测试资产，不得静默改变 P3C v1 的离线结果。

## 结果

安装不会因为处理插件包而隐式联网，也不会接受弱证书链或不可判定的撤销状态。代价是首次安装可能因本机尚无中间证书或有效撤销缓存而失败，且证书过期后即使包带时间戳也不能安装；这是 P3C 基础阶段有意选择的保守可回滚边界。

## 回滚

移除额外链标志和强签名参数可恢复此前行为，但不得在没有替代网络超时、隐私、撤销未知和时间戳验证设计时这样做。该策略不改变包格式、版本存储或已安装插件运行时。
