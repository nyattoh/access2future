param([string]$SourcePath, [string]$WorkDirectory, [switch]$Capabilities, [switch]$CleanupOwned)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'vba-analysis.ps1')
if ($CleanupOwned) {
  try {
    $owner=Get-Content -LiteralPath (Join-Path $WorkDirectory 'access-owner.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $process=Get-Process -Id ([int]$owner.pid) -ErrorAction Stop
    if ($process.ProcessName -eq 'MSACCESS' -and [string]$process.StartTime.ToUniversalTime().Ticks -eq [string]$owner.startedAtTicks) { Stop-Process -InputObject $process -Force; [void]$process.WaitForExit(3000) }
  } catch {}
  exit 0
}
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$app = $null; $db = $null; $temporaryDb = $null
$assets = [Collections.Generic.List[object]]::new()
$relations = [Collections.Generic.List[object]]::new()
$limitations = [Collections.Generic.List[object]]::new()
$vbaSourceFacts = [Collections.Generic.List[object]]::new()
function Issue([string]$code, [string]$message) { return @{code=$code;message=$message} }
function Add-Limitation($asset, [string]$code, [string]$message) {
  if (@($asset.issues | Where-Object { $_.code -eq $code -and $_.message -eq $message }).Count -gt 0) { return }
  $asset.status = 'partial'; $asset.issues += (Issue $code $message)
  $limitations.Add(@{code=$code;message=$message;assetId=$asset.id})
}
function Add-ObjectContainerFailure([string]$kind) {
  if ($kind -eq 'page') {
    $limitations.Add(@{code='DATA_ACCESS_PAGES_UNAVAILABLE';message='Access Data Access Pages の一覧を取得できませんでした。不在と取得失敗を区別できないため、ページがないとは判断しません。'})
    return
  }
  $limitations.Add(@{code='OBJECT_CONTAINER_UNAVAILABLE';message='保存オブジェクト一覧の一部を取得できませんでした。取得できた資産だけを表示します。'})
}
function New-Asset([string]$kind, [string]$name) { return @{id="${kind}:$name";kind=$kind;name=$name;caption=$name;dependsOn=@();status='supported';issues=@()} }
function Property($object,[string]$name,$default='') { try { return $object.Properties.Item($name).Value } catch { return $default } }
function Assert-NotCancelled {
  if ($WorkDirectory -and (Test-Path -LiteralPath (Join-Path $WorkDirectory 'cancel-requested'))) { throw [OperationCanceledException]::new() }
}
# 進捗は段階と件数だけを同じディレクトリの一時ファイルから置換する。
function Write-ProgressSnapshot([string]$phase, [long]$completed, $total) {
  if (-not $WorkDirectory -or $Capabilities) { return }
  $totalJson = if ($null -eq $total) { 'null' } else { [string]$total }
  $json = '{"phase":"' + $phase + '","completed":' + $completed + ',"total":' + $totalJson + '}'
  $progressPath=Join-Path $WorkDirectory 'progress.json'
  $temporaryPath=Join-Path $WorkDirectory 'progress.json.tmp'
  try {
    [IO.File]::WriteAllText($temporaryPath, $json, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporaryPath -Destination $progressPath -Force
  } catch { } # 進捗報告の失敗が解析を止めないようにする。
}
function Write-WorkerState([string]$operation, [int]$ordinal=0) {
  if (-not $WorkDirectory -or $Capabilities) { return }
  $path=Join-Path $WorkDirectory 'worker-state.json'
  $temporary=Join-Path $WorkDirectory 'worker-state.json.tmp'
  $state=@{operation=$operation;ordinal=$ordinal;recordedAt=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()}
  try { [IO.File]::WriteAllText($temporary, ($state | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false)); Move-Item -LiteralPath $temporary -Destination $path -Force } catch { }
}
function Add-DirectReference($asset, [string]$name, [string[]]$kinds) {
  $name = $name.Trim().Trim('[',']')
  if ($name -notmatch '^[\p{L}\p{N}_ .-]+$') { return }
  $known=@($assets | Where-Object { $_.name -eq $name -and $_.kind -in $kinds })
  if ($known.Count -eq 1) { $asset.dependsOn += $known[0].id }
  elseif ($known.Count -gt 1) { Add-Limitation $asset 'AMBIGUOUS_REFERENCE' ("参照先 " + $name + " は複数の資産候補に一致しました。") }
  elseif ($known.Count -eq 0 -and $asset.dependsOn -notcontains "external:$name") {
    $asset.dependsOn += "external:$name"
    Add-Limitation $asset 'UNRESOLVED_REFERENCE' ("定義内の参照先 " + $name + " を取得済みの資産で確認できませんでした。")
  }
}
function Find-Dependencies([string]$definition, $asset) {
  # ponytail: conservative static name matching; dynamic references always require review.
  # SQL の外部DB指定（IN '<file>'）を名前照合より先に検出する。同名でも外部表でありローカル資産へは結合しない。
  $externalNames=@{}
  $externalDatabases=@{}
  foreach ($match in [regex]::Matches($definition, '(?i)\b(?:FROM|JOIN|UPDATE|INSERT\s+INTO)\s+(?:\[([^\]\r\n]+)\]|([\p{L}\p{N}_]+))\s+IN\s+(''(?:[^'']|'''')*''|"(?:[^"]|"")*"|\[[^\]\r\n]+\])')) {
    $name=$match.Groups[1].Value; if (-not $name) { $name=$match.Groups[2].Value }
    $externalNames[$name]=$true
    $inner=$match.Groups[3].Value.Trim("'").Trim('"').Trim('[',']')
    $basename=[IO.Path]::GetFileName($inner)
    if ($basename -match '^[\p{L}\p{N}_(). -]+\.[\p{L}\p{N}]+$') { $externalDatabases[$basename]=$true }
  }
  $hasExternal=$externalNames.Count -gt 0
  if ($asset.kind -notin @('form','report','module')) {
    foreach ($candidate in $assets) {
      if ($candidate.id -eq $asset.id -or $candidate.kind -notin @('table','query','form','report','macro','module')) { continue }
      if ($hasExternal -and ($externalNames.ContainsKey($candidate.name) -or $candidate.kind -in @('table','query'))) { continue }
      $escaped = [regex]::Escape($candidate.name)
      if ($definition -match "(?i)(?<![\p{L}\p{N}_])(?:\[$escaped\]|$escaped)(?![\p{L}\p{N}_])") { $asset.dependsOn += $candidate.id }
    }
  }
  if ($hasExternal) {
    foreach ($name in @($externalNames.Keys)) { $asset.dependsOn += "external:$name" }
    $databases=(@($externalDatabases.Keys) -join '、'); if (-not $databases) { $databases='不明' }
    Add-Limitation $asset 'EXTERNAL_SQL_REFERENCE' ("SQL の IN 句が外部DBを参照しています（接続先: " + $databases + "、対象: " + (@($externalNames.Keys) -join '、') + "）。外部データは取得しておらず、同名を含むローカルの表・クエリへの結合も行っていません。IN 句を含む定義では他の表名・クエリ名も外部由来の可能性があるため静的な結合を保留しています。")
  }
  $sqlTexts=@()
  if ($asset.kind -eq 'query') { $sqlTexts += $definition }
  if ($asset.kind -in @('form','report')) {
    foreach ($match in [regex]::Matches($definition, '(?im)^\s*(?:RecordSource|RowSource)\s*=\s*"((?:[^"\r\n]|"")*)"')) {
      $value=$match.Groups[1].Value.Replace('""','"').Trim()
      if ($value) { $sqlTexts += $value }
    }
    foreach ($match in [regex]::Matches($definition, '(?im)^\s*SourceObject\s*=\s*"((?:[^"\r\n]|"")*)"')) {
      $value=$match.Groups[1].Value.Replace('""','"')
      $value=$value -replace '^(?i)(?:Form|Report|Query|フォーム|レポート|クエリ)\.',''
      if ($value.Trim()) { Add-DirectReference $asset $value @('form','report') }
    }
  }
  if (-not $hasExternal) {
    foreach ($sql in $sqlTexts) {
      if ($sql -match '(?i)^\s*(SELECT|INSERT|UPDATE|DELETE|TRANSFORM|PARAMETERS|WITH|UNION|EXEC|CREATE|DROP|ALTER)\b') {
        foreach ($match in [regex]::Matches($sql, '(?i)\b(?:FROM|JOIN|UPDATE|INSERT\s+INTO)\s+(?:\[([^\]\r\n]+)\]|([\p{L}\p{N}_]+))')) {
          $name=$match.Groups[1].Value; if (-not $name) { $name=$match.Groups[2].Value }
          Add-DirectReference $asset $name @('table','query')
        }
      } else {
        Add-DirectReference $asset $sql @('table','query')
      }
    }
  }
  $asset.dependsOn = @($asset.dependsOn | Select-Object -Unique)
  if ($definition -match '(?i)\b(Eval|CallByName|OpenRecordset|RunCode)\b|\[Event Procedure\]') {
    Add-Limitation $asset 'DYNAMIC_REFERENCE' '動的参照・イベント処理の依存は静的抽出だけでは確定できません。'
  }
}
function Get-AccessModuleSource($application,$asset) {
  $opened=$false; $module=$null; $object=$null; $objectType=0
  try {
    if ($asset.kind -eq 'form') {
      $objectType=2; $application.DoCmd.OpenForm($asset.name,1); $opened=$true; $object=$application.Forms.Item($asset.name)
      if (-not [bool]$object.HasModule) { $asset.hasCodeModule=$false; return '' }
      $asset.moduleType='class'; $module=$object.Module
    } elseif ($asset.kind -eq 'report') {
      $objectType=3; $application.DoCmd.OpenReport($asset.name,1); $opened=$true; $object=$application.Reports.Item($asset.name)
      if (-not [bool]$object.HasModule) { $asset.hasCodeModule=$false; return '' }
      $asset.moduleType='class'; $module=$object.Module
    } elseif ($asset.kind -eq 'module') {
      $objectType=5; $application.DoCmd.OpenModule($asset.name); $opened=$true; $module=$application.Modules.Item($asset.name)
      $asset.moduleType=if ([int]$module.Type -eq 1) { 'class' } else { 'standard' }
    } else { return '' }
    $asset.hasCodeModule=$true
    $count=[int]$module.CountOfLines
    if ($count -le 0) { return '' }
    return [string]$module.Lines(1,$count)
  } catch {
    Add-Limitation $asset 'VBA_SOURCE_UNAVAILABLE' 'VBAソースを安全に取得できませんでした。'
    return $null
  } finally {
    if ($module) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($module) }
    if ($object) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($object) }
    if ($opened) { try { $application.DoCmd.Close($objectType,$asset.name,2) } catch { } }
  }
}
try {
  Write-ProgressSnapshot 'preparing' 0 $null
  Assert-NotCancelled
  Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class AccessOwnedWindow { [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid); }'
  Assert-NotCancelled
  Write-WorkerState 'application-create'
  try { $app = New-Object -ComObject Access.Application } catch {
    if ($Capabilities) { @{available=$false} | ConvertTo-Json -Compress } else { @{error=@{code='ACCESS_UNAVAILABLE'}} | ConvertTo-Json -Compress }
    exit 0
  }
  [uint32]$ownedPid = 0
  [void][AccessOwnedWindow]::GetWindowThreadProcessId([IntPtr]$app.hWndAccessApp(), [ref]$ownedPid)
  if ($WorkDirectory -and $ownedPid -gt 0) {
    $owned=Get-Process -Id $ownedPid
    @{pid=$ownedPid;startedAtTicks=[string]$owned.StartTime.ToUniversalTime().Ticks;windowHandle=[string]$app.hWndAccessApp()} | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $WorkDirectory 'access-owner.json') -Encoding UTF8
  }
  Assert-NotCancelled
  $app.Visible = $false
  $app.AutomationSecurity = 3
  if ($Capabilities) { @{available=$true} | ConvertTo-Json -Compress; exit 0 }
  # The supplied DB is never the application's current DB. DAO reads metadata only.
  Write-WorkerState 'database-open'
  $db = $app.DBEngine.OpenDatabase($SourcePath, $false, $true)
  Write-ProgressSnapshot 'enumerating' 0 $null
  Write-WorkerState 'enumerate-tables'
  foreach ($table in $db.TableDefs) {
    Assert-NotCancelled
    if ($table.Name -match '^(MSys|~)') { continue }
    $asset = New-Asset 'table' ([string]$table.Name)
    $asset.linked=[bool]([string]$table.Connect)
    $asset.fields = @()
    if ([string]$table.Connect) {
      $connect=[string]$table.Connect
      if ($connect -notmatch '(?i)^ODBC;' -and $connect -match '(?i)(?:^|;)DATABASE=([^;]+)') {
        $databaseName=[IO.Path]::GetFileName($Matches[1])
        if ([IO.Path]::GetExtension($databaseName) -in @('.mdb','.accdb')) { $asset.linkedDatabaseName=$databaseName; $asset.linkedTableName=[string]$table.SourceTableName }
      }
      $asset.status='unsupported'
      $asset.issues += (Issue 'EXTERNAL_LINK_NOT_READ' '外部リンクのスキーマと行は取得していません。')
      $externalId = "external:$($table.Name)"
      $asset.dependsOn = @($externalId)
      $external = New-Asset 'external' ([string]$table.Name); $external.id=$externalId; $external.status='unsupported'; $external.issues=$asset.issues
      $assets.Add($external)
      $limitations.Add(@{code='EXTERNAL_LINK_NOT_READ';message='外部接続先は追跡していません。';assetId=$asset.id})
    } else {
      $primaryFields = @()
      foreach ($index in $table.Indexes) { if ($index.Primary) { foreach ($field in $index.Fields) { $primaryFields += [string]$field.Name } } }
      foreach ($field in $table.Fields) { $asset.fields += @{name=[string]$field.Name;dataType=[string]$field.Type;required=[bool]$field.Required;isPrimaryKey=([string]$field.Name -in $primaryFields)} }
    }
    $assets.Add($asset)
  }
  Write-WorkerState 'enumerate-queries'
  foreach ($query in $db.QueryDefs) {
    Assert-NotCancelled
    if ($query.Name -match '^~') { continue }
    $asset=New-Asset 'query' ([string]$query.Name)
    if ([string]$query.Connect) { Add-Limitation $asset 'EXTERNAL_QUERY_NOT_RUN' '外部接続クエリを実行していません。' }
    $assets.Add($asset)
  }
  $objectGroups=@(@{container='Forms';kind='form';type=2},@{container='Reports';kind='report';type=3},@{container='Scripts';kind='macro';type=4},@{container='Modules';kind='module';type=5},@{container='DataAccessPages';kind='page';type=6})
  Write-WorkerState 'enumerate-objects'
  foreach ($group in $objectGroups) {
    try {
      foreach ($document in $db.Containers.Item($group.container).Documents) {
        $asset=New-Asset $group.kind ([string]$document.Name)
        if ($group.kind -eq 'page') { $asset.status='unsupported'; $asset.issues+= (Issue 'LEGACY_PAGE' 'レガシーページは取得できません。') }
        $assets.Add($asset)
      }
    } catch { Add-ObjectContainerFailure $group.kind }
  }
  Write-WorkerState 'enumerate-relations'
  foreach ($relation in $db.Relations) {
    if ($relation.Table -match '^(MSys|~)' -or $relation.ForeignTable -match '^(MSys|~)') { continue }
    $fields=@(); foreach ($field in $relation.Fields) { $fields+=@{from=[string]$field.ForeignName;to=[string]$field.Name} }
    $relations.Add(@{from="table:$($relation.ForeignTable)";to="table:$($relation.Table)";fields=$fields;enforced=(($relation.Attributes -band 2) -eq 0)})
  }
  # 資産一覧が確定した時点で分母を固定する。以後の completed は「確認処理を終えた資産数」。
  $totalAssets = $assets.Count
  $confirmedAssets = 0
  Write-ProgressSnapshot 'analysing' 0 $totalAssets
  foreach ($asset in @($assets | Where-Object { $_.kind -notin @('form','report','macro','module') })) {
    Assert-NotCancelled
    Write-WorkerState 'static-definition' ($confirmedAssets + 1)
    if ($asset.kind -eq 'query') {
      $query=$db.QueryDefs.Item($asset.name)
      if (-not [string]$query.Connect) { Find-Dependencies ([string]$query.SQL) $asset; Add-Limitation $asset 'STATIC_DEPENDENCIES' 'SQL の依存は静的な名前照合で抽出しています。' }
    }
    $confirmedAssets++
    Write-ProgressSnapshot 'analysing' $confirmedAssets $totalAssets
  }
  $temporaryDb=Join-Path $WorkDirectory 'definitions.accdb'
  Write-WorkerState 'database-create'
  $app.NewCurrentDatabase($temporaryDb, 12)
  foreach ($asset in @($assets | Where-Object { $_.kind -in @('form','report','macro','module') })) {
    Assert-NotCancelled
    $type=($objectGroups | Where-Object { $_.kind -eq $asset.kind }).type
    $textPath=Join-Path $WorkDirectory ('definition-'+[Guid]::NewGuid().ToString('N')+'.txt')
    try {
      # Import definitions into an already open blank DB, never open any imported object.
      Write-WorkerState 'transfer-definition' ($confirmedAssets + 1)
      $app.DoCmd.TransferDatabase(0, 'Microsoft Access', $SourcePath, $type, $asset.name, $asset.name, $true, $false)
      Write-WorkerState 'export-definition' ($confirmedAssets + 1)
      $app.SaveAsText($type, $asset.name, $textPath)
      Write-WorkerState 'parse-definition' ($confirmedAssets + 1)
      $definition=[IO.File]::ReadAllText($textPath)
      Find-Dependencies $definition $asset
      if ($definition -match '(?m)^\s*Caption\s*=\s*"((?:[^"\r\n]|"")*)"') { $asset.caption=$Matches[1].Replace('""','"') }
      if ($asset.kind -in @('form','report','module')) {
        $source=Get-AccessModuleSource $app $asset
        if ($null -ne $source) {
          $facts=Get-VbaFacts $source
          $asset.procedureNames=@($facts.procedureNames)
          $asset.procedureCount=$facts.procedureNames.Count
          $vbaSourceFacts.Add(@{asset=$asset;source=$source;facts=$facts})
        }
      }
      Add-Limitation $asset 'STATIC_DEFINITION_ONLY' '保存定義のみ解析しました。起動・イベント・VBA は実行していません。'
    } catch { Add-Limitation $asset 'DEFINITION_UNAVAILABLE' '定義を安全に取得できませんでした。' }
    finally { if (Test-Path -LiteralPath $textPath) { Remove-Item -LiteralPath $textPath -Force } }
    $confirmedAssets++
    Write-ProgressSnapshot 'analysing' $confirmedAssets $totalAssets
  }
  $procedureIndex=@{}
  foreach ($entry in $vbaSourceFacts) {
    foreach ($procedure in $entry.facts.procedureNames) {
      if (-not $procedureIndex.ContainsKey($procedure)) { $procedureIndex[$procedure]=[Collections.Generic.List[string]]::new() }
      $procedureIndex[$procedure].Add([string]$entry.asset.id)
    }
  }
  foreach ($entry in $vbaSourceFacts) { [void](Find-VbaDependencies $entry.source $entry.asset $procedureIndex) }
  $limitations.Add(@{code='ROWS_NOT_READ';message='ローカル行を含むテーブルデータは読み出していません。'})
  Write-ProgressSnapshot 'finishing' $totalAssets $totalAssets
  Write-WorkerState 'emit-inventory'
  @{schemaVersion=1;source=@{accessVersion=[string]$app.Version};assets=@($assets.ToArray());relations=@($relations.ToArray());limitations=@($limitations.ToArray())} | ConvertTo-Json -Depth 30 -Compress
} catch {
  $code=if ($_.Exception -is [OperationCanceledException]) { 'ACCESS_CANCELLED' } else { 'ACCESS_EXTRACTION_FAILED' }
  @{error=@{code=$code}} | ConvertTo-Json -Compress
}
finally {
  Write-WorkerState 'cleanup'
  if ($db) { try { $db.Close() } catch { }; [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($db) }
  if ($app) { try { $app.CloseCurrentDatabase() } catch { }; try { $app.Quit(2) } catch { }; [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($WorkDirectory) { $pidPath=Join-Path $WorkDirectory 'access-owner.json'; if (Test-Path -LiteralPath $pidPath) { Remove-Item -LiteralPath $pidPath -Force } }
}
