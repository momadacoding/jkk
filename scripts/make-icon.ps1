<#
.SYNOPSIS
    生成应用图标的源图（1024x1024 PNG）。

.DESCRIPTION
    只产出**源图**。全套平台图标（.ico / .icns / 各尺寸 PNG）由
    `pnpm tauri icon <源图>` 从它派生，不要手工去改 src-tauri/icons/ 里的文件。

    设计：深色圆角方块 + 亮蓝 #4fc3f7 字样，与应用内深色主题和强调色一致。
    整块留了内边距——Windows 任务栏和开始菜单会再套一层自己的间距，
    图标画到边会显得比旁边的应用大一圈。

.EXAMPLE
    pwsh -File scripts/make-icon.ps1
    pnpm tauri icon src-tauri/icons/source.png
#>
[CmdletBinding()]
param(
    [string]$OutFile = "$PSScriptRoot/../src-tauri/icons/source.png",
    [int]$Size = 1024,
    [string]$Text = 'jkk'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# 圆角方块占画布的比例。留白交给系统去用，见上面的说明
$inset = [int]($Size * 0.06)
$side = $Size - 2 * $inset
$radius = [int]($side * 0.22)

$bmp = New-Object System.Drawing.Bitmap($Size, $Size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
try {
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.Clear([System.Drawing.Color]::Transparent)

    # --- 圆角方块 ---
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $radius * 2
    $path.AddArc($inset, $inset, $d, $d, 180, 90)
    $path.AddArc($inset + $side - $d, $inset, $d, $d, 270, 90)
    $path.AddArc($inset + $side - $d, $inset + $side - $d, $d, $d, 0, 90)
    $path.AddArc($inset, $inset + $side - $d, $d, $d, 90, 90)
    $path.CloseFigure()

    # 竖向渐变：纯平色在浅色背景上会糊成一团黑
    $rect = New-Object System.Drawing.Rectangle($inset, $inset, $side, $side)
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        $rect,
        [System.Drawing.ColorTranslator]::FromHtml('#2c333b'),
        [System.Drawing.ColorTranslator]::FromHtml('#15181c'),
        90.0)
    $g.FillPath($brush, $path)

    # 细边框，让图标在深色任务栏上仍有轮廓
    $pen = New-Object System.Drawing.Pen(
        [System.Drawing.ColorTranslator]::FromHtml('#3d454e'),
        [float]($Size * 0.006))
    $g.DrawPath($pen, $path)

    # --- 字样 ---
    # 用 GraphicsPath 而不是 DrawString：字形转成矢量路径后由 FillPath 抗锯齿，
    # 缩到 16x16 时边缘比 GDI 的位图字体渲染干净
    $family = New-Object System.Drawing.FontFamily('Segoe UI')
    $textPath = New-Object System.Drawing.Drawing2D.GraphicsPath
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = [System.Drawing.StringAlignment]::Center
    $fmt.LineAlignment = [System.Drawing.StringAlignment]::Center
    $textPath.AddString(
        $Text,
        $family,
        [int][System.Drawing.FontStyle]::Bold,
        [float]($side * 0.46),
        (New-Object System.Drawing.RectangleF($inset, $inset, $side, $side)),
        $fmt)
    $textBrush = New-Object System.Drawing.SolidBrush(
        [System.Drawing.ColorTranslator]::FromHtml('#4fc3f7'))
    $g.FillPath($textBrush, $textPath)

    $dir = Split-Path -Parent $OutFile
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $bmp.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output "wrote $OutFile ($Size x $Size)"
}
finally {
    foreach ($o in $g, $bmp, $path, $brush, $pen, $textPath, $textBrush, $fmt, $family) {
        if ($o) { $o.Dispose() }
    }
}
