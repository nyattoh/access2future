# Synthetic regression tests for dependency extraction in scripts/export-access.ps1.
# Only function definitions are parsed out of the script, so no COM side effects run.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$failures = 0
function Check([bool]$condition, [string]$label) {
  if ($condition) { Write-Output "PASS $label" } else { Write-Output "FAIL $label"; $script:failures = $script:failures + 1 }
}
$tokens = $null; $parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path (Join-Path $root 'scripts') 'export-access.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors -and $parseErrors.Count -gt 0) { Write-Output 'FAIL export-access.ps1 has parse errors'; exit 1 }
foreach ($functionAst in $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
  Invoke-Expression $functionAst.Extent.Text
}
$vbaAnalysisPath = Join-Path (Join-Path $root 'scripts') 'vba-analysis.ps1'
if (Test-Path -LiteralPath $vbaAnalysisPath) { . $vbaAnalysisPath }

# Counter-example 3 (P1): external IN clause must not bind to the same-name local table.
$assets = [Collections.Generic.List[object]]::new(); $limitations = [Collections.Generic.List[object]]::new()
[void]$assets.Add((New-Asset 'table' 'Orders')); [void]$assets.Add((New-Asset 'table' 'Customers'))
$query = New-Asset 'query' 'Q'
Find-Dependencies "SELECT * FROM Orders IN 'C:\synthetic-external\other.accdb'" $query
Check ($query.dependsOn -notcontains 'table:Orders') 'external-IN does not bind same-name local table'
Check ($query.dependsOn -contains 'external:Orders') 'external-IN keeps concrete unresolved external reference'
Check ($query.dependsOn -notcontains 'table:Customers') 'external-IN freezes other table/query bindings in the definition'
$externalIssues = @($query.issues | Where-Object { $_.code -eq 'EXTERNAL_SQL_REFERENCE' })
Check ($externalIssues.Count -eq 1) 'external-IN records an issue'
$externalMessage = ($externalIssues | Select-Object -First 1).message
Check ($externalMessage -match 'other\.accdb') 'issue records connection basename only'
$serialized = (ConvertTo-Json $query -Depth 6) + (ConvertTo-Json $limitations.ToArray() -Depth 6)
Check ($serialized -notmatch 'synthetic-external|C:\\') 'no raw external path in issue or limitation output'

# Counter-example 4a (P2): unknown RecordSource must stay visible instead of vanishing.
$assets = [Collections.Generic.List[object]]::new(); $limitations = [Collections.Generic.List[object]]::new()
[void]$assets.Add((New-Asset 'query' 'ExistingQuery'))
$form = New-Asset 'form' 'Main'
Find-Dependencies 'RecordSource = "MissingQuery"' $form
Check ($form.dependsOn -contains 'external:MissingQuery') 'unknown RecordSource stays as unresolved reference'
Check (@($form.issues | Where-Object { $_.code -eq 'UNRESOLVED_REFERENCE' }).Count -ge 1) 'unknown RecordSource records an issue'
Check (@($form.issues | Where-Object { $_.code -eq 'UNRESOLVED_REFERENCE' } | ForEach-Object { $_.message } | Select-String 'MissingQuery').Count -ge 1) 'issue names the missing reference'
$form2 = New-Asset 'form' 'Main2'
Find-Dependencies 'RecordSource ="ExistingQuery"' $form2
Check ($form2.dependsOn -contains 'query:ExistingQuery') 'known RecordSource resolves to the asset'

# Counter-example 4b (P2): UPDATE target and other SQL references must stay visible.
$assets = [Collections.Generic.List[object]]::new(); $limitations = [Collections.Generic.List[object]]::new()
[void]$assets.Add((New-Asset 'table' 'Customers'))
$query2 = New-Asset 'query' 'UpdateQ'
Find-Dependencies 'UPDATE MissingTable SET ID=1' $query2
Check ($query2.dependsOn -contains 'external:MissingTable') 'unknown UPDATE target stays as unresolved reference'
Check (@($query2.issues | Where-Object { $_.code -eq 'UNRESOLVED_REFERENCE' }).Count -ge 1) 'unknown UPDATE target records an issue'
$query3 = New-Asset 'query' 'UpdateKnown'
Find-Dependencies 'UPDATE Customers SET ID=1' $query3
Check ($query3.dependsOn -contains 'table:Customers') 'known UPDATE target resolves to the asset'

# Direct references in forms: RowSource SQL fragments and SourceObject subform references.
$form3 = New-Asset 'form' 'Host'
Find-Dependencies 'RowSource ="SELECT Code FROM MissingLookup"' $form3
Check ($form3.dependsOn -contains 'external:MissingLookup') 'unknown RowSource SQL reference stays unresolved'
$form4 = New-Asset 'form' 'Host2'
Find-Dependencies 'SourceObject ="MissingSub"' $form4
Check ($form4.dependsOn -contains 'external:MissingSub') 'unknown SourceObject stays as unresolved reference'
$form5 = New-Asset 'form' 'Host3'
[void]$assets.Add((New-Asset 'form' 'ExistingSub'))
Find-Dependencies 'SourceObject ="ExistingSub"' $form5
Check ($form5.dependsOn -contains 'form:ExistingSub') 'known SourceObject resolves to the subform asset'
Check ($form5.dependsOn -notcontains 'external:ExistingSub') 'known SourceObject is not left unresolved'

# Non-regression: value lists and dynamic references keep their existing behavior.
$form6 = New-Asset 'form' 'Host4'
Find-Dependencies 'RowSource ="Yes;No"' $form6
Check ($form6.dependsOn.Count -eq 0 -and $form6.issues.Count -eq 0) 'value list RowSource adds no dependency or issue'
$form7 = New-Asset 'form' 'Host5'
Find-Dependencies 'OnClick ="[Event Procedure]"' $form7
Check (@($form7.issues | Where-Object { $_.code -eq 'DYNAMIC_REFERENCE' }).Count -eq 1) 'dynamic reference note is still recorded'
$module = New-Asset 'module' 'TaxRules'
Find-Dependencies 'Public Function TaxRate(ByVal amount As Currency) As Double' $module
Check (@($module.issues | Where-Object { $_.code -eq 'DYNAMIC_REFERENCE' }).Count -eq 0) 'ordinary VBA procedure declaration alone is not called a dynamic reference'

# Ambiguous names and object-container failures must remain visible without opening an Access DB.
$assets = [Collections.Generic.List[object]]::new(); $limitations = [Collections.Generic.List[object]]::new()
[void]$assets.Add((New-Asset 'form' 'SharedName')); [void]$assets.Add((New-Asset 'report' 'SharedName'))
$sourceAsset = New-Asset 'form' 'Host'
Add-DirectReference $sourceAsset 'SharedName' @('form','report')
Check (@($sourceAsset.issues | Where-Object { $_.code -eq 'AMBIGUOUS_REFERENCE' }).Count -eq 1) 'same-name form/report reference records ambiguity'
Check ($sourceAsset.dependsOn.Count -eq 0) 'ambiguous direct reference is not resolved to a single asset'

$handlerAvailable = [bool](Get-Command Add-ObjectContainerFailure -CommandType Function -ErrorAction SilentlyContinue)
if ($handlerAvailable) {
  $limitations = [Collections.Generic.List[object]]::new()
  Add-ObjectContainerFailure 'report'
  Check (@($limitations | Where-Object { $_.code -eq 'OBJECT_CONTAINER_UNAVAILABLE' }).Count -eq 1) 'required object-container failure is visible'
  $limitations.Clear()
  Add-ObjectContainerFailure 'page'
  Check (@($limitations | Where-Object { $_.code -eq 'DATA_ACCESS_PAGES_UNAVAILABLE' }).Count -eq 1) 'Data Access Pages enumeration failure is not reported as an empty inventory'
} else {
  Check $false 'object-container failure handler is present for synthetic testing'
}

# VBA source is untrusted text. Only executable statements may create dependency candidates.
if (Get-Command Get-VbaFacts -CommandType Function -ErrorAction SilentlyContinue) {
  $vba = @'
Option Explicit
Public Sub CreateInvoice()
  DoCmd.OpenReport "Invoice"
  CurrentDb.Execute "UPDATE InvoiceHeaders SET Printed = True"
  Call RecalculateInvoice
  DoCmd.OpenForm TargetFormName
  Debug.Print "DoCmd.OpenForm ""GhostForm"""
  ' DoCmd.RunMacro "CommentMacro"
End Sub
Private Sub Form_Load()
  Call RecalculateInvoice
End Sub
'@
  $facts = Get-VbaFacts $vba
  Check ($facts.procedureNames -contains 'CreateInvoice' -and $facts.procedureNames -contains 'Form_Load') 'VBA procedures and form event handlers are extracted'
  Check (@($facts.objectReferences | Where-Object { $_.kind -eq 'report' -and $_.name -eq 'Invoice' }).Count -eq 1) 'literal OpenReport target is a dependency candidate'
  Check (@($facts.objectReferences | Where-Object { $_.kind -eq 'table-or-query' -and $_.name -eq 'InvoiceHeaders' }).Count -eq 1) 'literal SQL target is a dependency candidate'
  Check ($facts.procedureCalls -contains 'RecalculateInvoice') 'explicit procedure call is a dependency candidate'
  $implicit = "Public Sub Wrapper()`r`n  RecalculateInvoice()`r`nEnd Sub"
  Check ((Get-VbaProcedureCallCandidates $implicit @{ RecalculateInvoice = @('module:InvoiceRules') }) -contains 'RecalculateInvoice') 'implicit function calls resolve against the project procedure index'
  $externalVba = 'Private Declare PtrSafe Function Sleep Lib "kernel32.dll" (ByVal ms As Long) As Long' + "`r`n" + 'Set excel = CreateObject("Excel.Application")'
  $externalFacts = Get-VbaFacts $externalVba
  Check (@($externalFacts.objectReferences | Where-Object { $_.kind -eq 'external' -and $_.name -eq 'kernel32.dll' }).Count -eq 1) 'Declare Lib exposes an external library dependency'
  Check (@($externalFacts.objectReferences | Where-Object { $_.kind -eq 'external' -and $_.name -eq 'Excel.Application' }).Count -eq 1) 'CreateObject exposes a static COM dependency'
  Check (@($facts.objectReferences | Where-Object { $_.name -in @('GhostForm','CommentMacro') }).Count -eq 0) 'comment and string contents do not create false dependencies'
  Check ($facts.dynamicReference -eq $true) 'variable form name is kept as an unresolved dynamic reference'
  $assets = [Collections.Generic.List[object]]::new(); $limitations = [Collections.Generic.List[object]]::new()
  [void]$assets.Add((New-Asset 'report' 'Invoice')); [void]$assets.Add((New-Asset 'table' 'InvoiceHeaders')); [void]$assets.Add((New-Asset 'module' 'InvoiceRules'))
  $caller = New-Asset 'form' 'InvoiceEntry'
  [void](Find-VbaDependencies $vba $caller @{ RecalculateInvoice = @('module:InvoiceRules') })
  Check ($caller.dependsOn -contains 'report:Invoice' -and $caller.dependsOn -contains 'table:InvoiceHeaders' -and $caller.dependsOn -contains 'module:InvoiceRules') 'VBA references resolve to object and module dependencies'
  Check (@($caller.issues | Where-Object { $_.code -eq 'DYNAMIC_REFERENCE' }).Count -eq 1) 'dynamic VBA reference is reported without execution'
  $ambiguousCaller=New-Asset 'form' 'AmbiguousCall'
  [void](Find-VbaDependencies 'Private Sub Click()
  Call DuplicateTask
End Sub' $ambiguousCaller @{DuplicateTask=@('module:First','module:Second')})
  Check (@($ambiguousCaller.issues | Where-Object { $_.code -eq 'AMBIGUOUS_PROCEDURE_REFERENCE' }).Count -eq 1) 'duplicate procedure names remain unresolved instead of choosing a module'
  $missingCaller=New-Asset 'form' 'MissingCall'
  [void](Find-VbaDependencies 'Private Sub Click()
  Call MissingTask
End Sub' $missingCaller @{})
  Check (@($missingCaller.issues | Where-Object { $_.code -eq 'UNRESOLVED_PROCEDURE_REFERENCE' }).Count -eq 1) 'procedure call without an extracted definition stays unresolved'
} else {
  Check $false 'VBA source analysis helper is available for synthetic testing'
}

if ($script:failures -gt 0) { Write-Output ("FAILED count=" + $script:failures); exit 1 }
Write-Output 'ALL PASS'
