# Aider-AI/aider 源码阅读卡片

采集日期：2026-10-13。静态资料整理，未安装或运行本项目。

- 主题：编程Agent
- 参考目标：仓库映射、编辑格式及 Git 操作
- 完整提交：`5dc9490bb35f9729ef2c95d00a19ccd30c26339c`
- 快照范围：curated text source snapshot
- 原始仓库：[GitHub](https://github.com/Aider-AI/aider)
- [本地源码 ZIP](../../project-snapshots/Aider-AI--aider-5dc9490bb35f.zip)
- 许可证识别：Apache-2.0
- 顶层许可文件：LICENSE.txt

## 离线阅读

[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)

[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。

### aider/io.py

[本地文件](selected-source/aider/io.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/io.py)

源码定位声明：

- `def ensure_hash_prefix(color):`
- `def restore_multiline(func):`
- `def wrapper(self, *args, **kwargs):`
- `class CommandCompletionException(Exception):`
- `class ConfirmGroup:`

### aider/gui.py

[本地文件](selected-source/aider/gui.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/gui.py)

源码定位声明：

- `class CaptureIO(InputOutput):`
- `def tool_output(self, msg, log_only=False):`
- `def tool_error(self, msg):`
- `def tool_warning(self, msg):`
- `def get_captured_lines(self):`

### aider/llm.py

[本地文件](selected-source/aider/llm.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/llm.py)

源码定位声明：

- `class LazyLiteLLM:`
- `def __getattr__(self, name):`
- `def _load_litellm(self):`

### aider/args.py

[本地文件](selected-source/aider/args.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/args.py)

源码定位声明：

- `def resolve_aiderignore_path(path_str, git_root=None):`
- `def default_env_file(git_root):`
- `def get_parser(default_config_files, git_root):`
- `def get_md_help():`
- `def get_sample_yaml():`

### aider/dump.py

[本地文件](selected-source/aider/dump.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/dump.py)

源码定位声明：

- `def cvt(s):`
- `def dump(*vals):`

### aider/help.py

[本地文件](selected-source/aider/help.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/help.py)

源码定位声明：

- `def install_help_extra(io):`
- `def get_package_files():`
- `def fname_to_url(filepath):`
- `def get_index():`
- `class Help:`

### aider/main.py

[本地文件](selected-source/aider/main.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/main.py)

源码定位声明：

- `def check_config_files_for_yes(config_files):`
- `def get_git_root():`
- `def guessed_wrong_repo(io, git_root, fnames, git_dname):`
- `def make_new_repo(git_root, io):`
- `def setup_git(git_root, io):`

### aider/repo.py

[本地文件](selected-source/aider/repo.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/repo.py)

源码定位声明：

- `def set_git_env(var_name, value, original_value):`
- `class GitRepo:`
- `def __init__(`
- `def commit(self, fnames=None, context=None, message=None, aider_edits=False, coder=None):`
- `def get_rel_repo_dir(self):`

### aider/urls.py

[本地文件](selected-source/aider/urls.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/urls.py)

### aider/diffs.py

[本地文件](selected-source/aider/diffs.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/diffs.py)

源码定位声明：

- `def main():`
- `def create_progress_bar(percentage):`
- `def assert_newlines(lines):`
- `def diff_partial_update(lines_orig, lines_updated, final=False, fname=None):`
- `def find_last_non_deleted(lines_orig, lines_updated):`

### aider/utils.py

[本地文件](selected-source/aider/utils.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/utils.py)

源码定位声明：

- `class IgnorantTemporaryDirectory:`
- `def __init__(self):`
- `def __enter__(self):`
- `def __exit__(self, exc_type, exc_val, exc_tb):`
- `def cleanup(self):`

### aider/voice.py

[本地文件](selected-source/aider/voice.py) · [固定提交来源](https://github.com/Aider-AI/aider/blob/5dc9490bb35f9729ef2c95d00a19ccd30c26339c/aider/voice.py)

源码定位声明：

- `class SoundDeviceError(Exception):`
- `class Voice:`
- `def __init__(self, audio_format="wav", device_name=None):`
- `def callback(self, indata, frames, time, status):`
- `def get_prompt(self):`

## 对照本项目时记录什么

看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。

第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。
