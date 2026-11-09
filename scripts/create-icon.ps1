Add-Type -AssemblyName System.Drawing
$bitmap = New-Object System.Drawing.Bitmap 1024,1024
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::FromArgb(255,25,26,33))
$accent = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255,190,174,247)),78
$accent.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$accent.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$graphics.DrawLine($accent,512,245,512,657)
$graphics.DrawLine($accent,344,495,512,663)
$graphics.DrawLine($accent,680,495,512,663)
$base = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255,137,129,159)),52
$base.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
$base.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
$graphics.DrawLine($base,290,790,734,790)
$target = Join-Path $PSScriptRoot '../src-tauri/icons/source.png'
$bitmap.Save($target,[System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()
$accent.Dispose()
$base.Dispose()
