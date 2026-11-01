# 候选结构巡检（待批阅）

检查当前桥接及基础候选 86 个后端轮次、14505 条观测，得到 256 个去重条目。

重复按文字+消费范围+分组判断；跨请求扫描可能受候选池扩容影响，不能直接当作同一静态池错误。组0包含合并词句区，不等同于纯句子。CLI在无分组时用3表示未分区，合法。未执行GUI。

|类型|条目数|
|---|---:|
|扫描分页出现重复候选元组|256|

## 示例（前30项，完整证据见 structure-issues.jsonl）

|ID|输入|类型|观测数|
|---|---|---|---:|
|structure-fb6cff70656a|a|扫描分页出现重复候选元组|3|
|structure-2647ee8dcd60|bagnzhu|扫描分页出现重复候选元组|2|
|structure-6bbdf1daa85d|bagzhu|扫描分页出现重复候选元组|1|
|structure-ef91efe6c195|baocnu|扫描分页出现重复候选元组|2|
|structure-2730bef61b7a|bc|扫描分页出现重复候选元组|1|
|structure-a08c4552e6fa|bgs|扫描分页出现重复候选元组|1|
|structure-7253f86e0601|bianmfuw|扫描分页出现重复候选元组|2|
|structure-ecc3e29489e1|bnagongshi|扫描分页出现重复候选元组|1|
|structure-0c322a89a9c3|bq|扫描分页出现重复候选元组|1|
|structure-17414380d40f|bugo|扫描分页出现重复候选元组|3|
|structure-c2767ec4ecdf|bx|扫描分页出现重复候选元组|1|
|structure-5ae34475677a|byetong|扫描分页出现重复候选元组|2|
|structure-b24f0109e5ef|bz|扫描分页出现重复候选元组|1|
|structure-92f6b0c2654b|cf|扫描分页出现重复候选元组|1|
|structure-77f9c14eda1d|cfan|扫描分页出现重复候选元组|2|
|structure-6af913417319|cheggong|扫描分页出现重复候选元组|2|
|structure-27fbafec3c26|chegngong|扫描分页出现重复候选元组|2|
|structure-011894e481a7|chiwanfanyihouzaichuqusanbu|扫描分页出现重复候选元组|1|
|structure-c74c241d5fab|chnag|扫描分页出现重复候选元组|4|
|structure-3f109e6bd89d|chnag|扫描分页出现重复候选元组|2|
|structure-2ab3f7e59c6d|chnag|扫描分页出现重复候选元组|2|
|structure-1f3c3e79ff65|choashi|扫描分页出现重复候选元组|3|
|structure-7dbe09eb91d2|chongxni|扫描分页出现重复候选元组|2|
|structure-46981d6bf443|cihfan|扫描分页出现重复候选元组|2|
|structure-8b5b3765ba8a|cj|扫描分页出现重复候选元组|1|
|structure-553e4ca6cf3e|ck|扫描分页出现重复候选元组|1|
|structure-16a85e0e1949|cm|扫描分页出现重复候选元组|1|
|structure-a29058290c95|cs|扫描分页出现重复候选元组|1|
|structure-1ffde406e460|cz|扫描分页出现重复候选元组|1|
|structure-1e60ecc298cf|czxt|扫描分页出现重复候选元组|1|

另有 358 项布局规则观察写入 structure-layout-observations.jsonl，不计为缺陷条目。
例如 bgs 的办公室等字典词可能共用组0；安全可以优先于更长的“按去按”组合。因此不能仅凭组0数量或位置认定违反“长句最多两个”规则。
不硬编码字典/单字区配额，因为补位、单字优先和词句合并会改变合法布局。
