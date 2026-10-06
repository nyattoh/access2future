# Pure, local-only static analysis for Access VBA source text. Never execute VBA.
function ConvertTo-VbaStatements([string]$source) {
  $result=[Collections.Generic.List[object]]::new()
  $pending=''; $values=[Collections.Generic.List[string]]::new(); $stringNumber=0
  foreach ($line in ($source -split '\r?\n')) {
    $code=[Text.StringBuilder]::new(); $literal=[Text.StringBuilder]::new(); $inside=$false
    for ($i=0; $i -lt $line.Length; $i++) {
      $char=$line[$i]
      if ($inside) {
        if ($char -eq '"') {
          if ($i + 1 -lt $line.Length -and $line[$i + 1] -eq '"') { [void]$literal.Append('"'); $i++ }
          else {
            $inside=$false; $values.Add($literal.ToString()); [void]$code.Append(('__STR{0}__' -f $stringNumber)); $stringNumber++; $literal.Clear() | Out-Null
          }
        } else { [void]$literal.Append($char) }
      } elseif ($char -eq '"') { $inside=$true }
      elseif ($char -eq "'") { break }
      else { [void]$code.Append($char) }
    }
    $part=$code.ToString()
    if (-not $inside -and $part.Trim() -match '(?i)^Rem(?:\s|$)') { $part='' }
    $pending += $part
    if (-not $inside -and $pending.TrimEnd() -match '\s_$') { $pending=[regex]::Replace($pending,'\s_$',' '); continue }
    if ($pending.Trim()) { $result.Add(@{code=$pending;strings=@($values.ToArray())}) }
    $pending=''; $values.Clear(); $stringNumber=0
  }
  if ($pending.Trim()) { $result.Add(@{code=$pending;strings=@($values.ToArray())}) }
  return $result.ToArray()
}

function Get-VbaFacts([string]$source) {
  $statements=@(ConvertTo-VbaStatements $source)
  $procedures=[Collections.Generic.List[string]]::new()
  $calls=[Collections.Generic.List[string]]::new()
  $references=[Collections.Generic.List[object]]::new()
  $dynamic=$false
  foreach ($statement in $statements) {
    $code=[string]$statement.code
    $proc=[regex]::Match($code,'(?i)^\s*(?:(?:Public|Private|Friend|Static)\s+)*(?:Sub|Function|Property\s+(?:Get|Let|Set))\s+([\p{L}_][\p{L}\p{N}_]*)\b')
    if ($proc.Success) { $procedures.Add($proc.Groups[1].Value) }
    foreach ($spec in @(@{method='OpenForm';kind='form'},@{method='OpenReport';kind='report'},@{method='OpenQuery';kind='query'},@{method='OpenTable';kind='table'},@{method='RunMacro';kind='macro'})) {
      $method=[regex]::Match($code,("(?i)\bDoCmd\.{0}\b" -f [regex]::Escape($spec.method)))
      if (-not $method.Success) { continue }
      $args=$code.Substring($method.Index + $method.Length)
      $literal=[regex]::Match($args,'^\s*(?:[A-Za-z_][\w]*\s*:=\s*)?__STR(\d+)__')
      if ($literal.Success) {
        $index=[int]$literal.Groups[1].Value
        if ($index -lt $statement.strings.Count -and $statement.strings[$index].Trim()) { $references.Add(@{kind=$spec.kind;name=$statement.strings[$index].Trim()}) }
      } else { $dynamic=$true }
    }
    foreach ($spec in @(@{pattern='(?i)\bForms\s*!\s*(?:\[([^\]]+)\]|([\p{L}\p{N}_]+))';kind='form'},@{pattern='(?i)\bReports\s*!\s*(?:\[([^\]]+)\]|([\p{L}\p{N}_]+))';kind='report'})) {
      foreach ($match in [regex]::Matches($code,$spec.pattern)) { $name=$match.Groups[1].Value; if (-not $name) { $name=$match.Groups[2].Value }; if ($name) { $references.Add(@{kind=$spec.kind;name=$name.Trim()}) } }
    }
    foreach ($spec in @(@{pattern='(?i)\bForms\s*\(\s*__STR(\d+)__\s*\)';kind='form'},@{pattern='(?i)\bReports\s*\(\s*__STR(\d+)__\s*\)';kind='report'})) {
      foreach ($match in [regex]::Matches($code,$spec.pattern)) { $index=[int]$match.Groups[1].Value; if ($index -lt $statement.strings.Count -and $statement.strings[$index].Trim()) { $references.Add(@{kind=$spec.kind;name=$statement.strings[$index].Trim()}) } }
    }
    foreach ($match in [regex]::Matches($code,'(?i)\bCall\s+(?:(?<module>[\p{L}_][\p{L}\p{N}_]*)\.)?(?<procedure>[\p{L}_][\p{L}\p{N}_]*)\b')) {
      $calls.Add($(if ($match.Groups['module'].Success) { $match.Groups['module'].Value + '.' + $match.Groups['procedure'].Value } else { $match.Groups['procedure'].Value }))
    }
    $domainCalls=[regex]::Matches($code,'(?i)\b(?:DLookup|DCount|DSum|DAvg|DMax|DMin|DFirst|DLast|DStDev|DStDevP|DVar|DVarP)\s*\(')
    if ($domainCalls.Count -and $code -notmatch '(?i)\b(?:DLookup|DCount|DSum|DAvg|DMax|DMin|DFirst|DLast|DStDev|DStDevP|DVar|DVarP)\s*\(\s*__STR\d+__\s*,\s*__STR\d+__') { $dynamic=$true }
    foreach ($match in [regex]::Matches($code,'(?i)\b(?:DLookup|DCount|DSum|DAvg|DMax|DMin|DFirst|DLast|DStDev|DStDevP|DVar|DVarP)\s*\(\s*__STR\d+__\s*,\s*__STR(\d+)__')) {
      $index=[int]$match.Groups[1].Value
      if ($index -lt $statement.strings.Count -and $statement.strings[$index].Trim()) { $references.Add(@{kind='table';name=$statement.strings[$index].Trim()}) }
      else { $dynamic=$true }
    }
    foreach($match in [regex]::Matches($code,'(?i)\b(?:CreateObject|GetObject)\s*\(\s*__STR(\d+)__')){
      $index=[int]$match.Groups[1].Value
      if($index -lt $statement.strings.Count -and $statement.strings[$index].Trim()){$references.Add(@{kind='external';name=$statement.strings[$index].Trim()})}else{$dynamic=$true}
    }
    foreach($match in [regex]::Matches($code,'(?i)\bLib\s+__STR(\d+)__')){
      $index=[int]$match.Groups[1].Value
      if($index -lt $statement.strings.Count -and $statement.strings[$index].Trim()){$library=[IO.Path]::GetFileName($statement.strings[$index].Trim().Trim('"'));if($library){$references.Add(@{kind='external';name=$library})}}else{$dynamic=$true}
    }
    $sqlContext=($code -match '(?i)\b(?:Execute|RunSQL|OpenRecordset)\s+__STR\d+__|\b(?:str)?sql\w*\s*=\s*__STR\d+__')
    foreach ($sql in @($statement.strings)) {
      if ($sqlContext -and $sql -match '(?i)^\s*(?:SELECT|INSERT|UPDATE|DELETE|TRANSFORM|PARAMETERS|WITH)\b') {
        foreach ($match in [regex]::Matches($sql,'(?i)\b(?:FROM|JOIN|UPDATE|INSERT\s+INTO|DELETE\s+FROM)\s+(?:\[([^\]\r\n]+)\]|([\p{L}\p{N}_]+))')) {
          $name=$match.Groups[1].Value; if (-not $name) { $name=$match.Groups[2].Value }
          if ($name -and $name -notmatch '^(?i:SELECT|SET|WHERE|VALUES|INTO)$') { $references.Add(@{kind='table-or-query';name=$name}) }
        }
      }
    }
    if ($code -match '(?i)\b(?:Eval|CallByName)\s*\(|\bApplication\.Run\b|\bDoCmd\.RunSQL\s+(?!__STR\d+__)|\b(?:CurrentDb\.)?Execute\s+(?!__STR\d+__)') { $dynamic=$true }
    if ($code -match '(?i)\b(?:DoCmd\.OpenForm|DoCmd\.OpenReport|DoCmd\.OpenQuery|DoCmd\.OpenTable|DoCmd\.RunMacro)\b' -and $code -match '&') { $dynamic=$true }
  }
  $uniqueReferences=@{}; foreach ($reference in $references) { $key=($reference.kind + ':' + $reference.name).ToLowerInvariant(); if (-not $uniqueReferences.ContainsKey($key)) { $uniqueReferences[$key]=$reference } }
  return @{procedureNames=@($procedures.ToArray() | Select-Object -Unique);procedureCalls=@($calls.ToArray() | Select-Object -Unique);objectReferences=@($uniqueReferences.Values);dynamicReference=$dynamic}
}

function Get-VbaProcedureCallCandidates([string]$source,$procedureIndex) {
  $candidates=[Collections.Generic.List[string]]::new()
  $known=@{}; foreach($procedure in $procedureIndex.Keys){$known[[string]$procedure]=[string]$procedure}
  foreach($statement in (ConvertTo-VbaStatements $source)){
    $code=[string]$statement.code
    if($code -match '(?i)^\s*(?:(?:Public|Private|Friend|Static)\s+)*(?:Sub|Function|Property\s+(?:Get|Let|Set))\s+[\p{L}_][\p{L}\p{N}_]*\b'){continue}
    foreach($match in [regex]::Matches($code,'(?i)(?<![\p{L}\p{N}_])(?<name>[\p{L}_][\p{L}\p{N}_]*)\s*\(')){$name=$match.Groups['name'].Value;if($known.ContainsKey($name)){$candidates.Add($known[$name])}}
    $bare=[regex]::Match($code,'(?i)^\s*(?<module>[\p{L}_][\p{L}\p{N}_]*\.)?(?<name>[\p{L}_][\p{L}\p{N}_]*)\s+(?![=:])')
    if($bare.Success){$name=$bare.Groups['name'].Value;if($known.ContainsKey($name)){$candidates.Add($known[$name])}}
  }
  return @($candidates.ToArray()|Select-Object -Unique)
}

function Find-VbaDependencies([string]$source,$asset,$procedureIndex) {
  $facts=Get-VbaFacts $source
  $asset.procedureNames=@($facts.procedureNames)
  $asset.procedureCount=$facts.procedureNames.Count
  foreach ($reference in $facts.objectReferences) {
    $kinds=switch ($reference.kind) {
      'table-or-query' { @('table','query') }
      default { @($reference.kind) }
    }
    Add-DirectReference $asset ([string]$reference.name) $kinds
  }
  $calls=@($facts.procedureCalls + (Get-VbaProcedureCallCandidates $source $procedureIndex) | Select-Object -Unique)
  foreach ($call in $calls) {
    $parts=$call.Split('.',2); $procedure=$parts[-1]
    if ($parts.Count -eq 2) {
      $moduleName=$parts[0]
      $modules=@($assets | Where-Object { $_.kind -eq 'module' -and $_.name -eq $moduleName })
      if ($modules.Count -eq 1) { if ($modules[0].id -ne $asset.id) { $asset.dependsOn += $modules[0].id }; continue }
    }
    $owners=@($procedureIndex[$procedure] | Select-Object -Unique)
    if ($owners.Count -eq 1) {
      if ($owners[0] -ne $asset.id) { $asset.dependsOn += [string]$owners[0] }
    } elseif ($owners.Count -gt 1) {
      Add-Limitation $asset 'AMBIGUOUS_PROCEDURE_REFERENCE' ("手続き " + $procedure + " の呼出し先が複数候補に一致しました。")
    } else {
      $asset.dependsOn += ('external:procedure:' + $procedure)
      Add-Limitation $asset 'UNRESOLVED_PROCEDURE_REFERENCE' ("呼出し先 " + $procedure + " の定義を取得済みの資産で確認できませんでした。")
    }
  }
  $asset.dependsOn=@($asset.dependsOn | Select-Object -Unique)
  if ($facts.dynamicReference) { Add-Limitation $asset 'DYNAMIC_REFERENCE' 'VBAの動的なオブジェクト名やSQLは静的抽出だけでは確定できません。' }
  return $facts
}
