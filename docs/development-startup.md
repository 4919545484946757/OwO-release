# 开发版启动

在项目根目录双击 `start-owo-dev.cmd`：编译当前工作区的 Release 内核和
x64/x86 TSF，重启 Core/ModelHost，注册开发版输入法并打开记事本。
已有应用可能仍加载旧 TSF；请在新窗口中测试，必要时按 Win+Space 选择 OwO。

入口脚本可从任意工作目录调用，并转发额外参数，例如：

```powershell
D:\0program\OwO\start-owo-dev.cmd -OpenSettings
D:\0program\OwO\start-owo-dev.cmd -DisableModelHost
```

启动 Core 不再隐式安装、授权或激活五笔插件，不影响现有插件的启用状态。
需要部署五笔开发插件时显式添加 `-DeployWubi86`；未签名开发包仍要求
明确了解风险并传入 `-AcceptDevelopmentPackageRisk`，普通启动不需要该参数。

不重新编译、仅启动已有开发构建：

```powershell
& D:\0program\OwO\scripts\start-dev.ps1 -BuildDirectory D:\0program\OwO\build\windows-release\Release -RestartCore -OpenNotepad
```

脚本会检查进程路径、输入法激活状态和 Core 管道是否就绪后才报告启动成功。
此流程会更新本机开发版注册，但不会打包或发布发行版。
