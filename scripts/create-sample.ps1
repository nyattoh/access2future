param([Parameter(Mandatory=$true)][string]$OutputPath)
$ErrorActionPreference='Stop'
$app=$null; $db=$null; $externalPath=$null
try {
  if (Test-Path -LiteralPath $OutputPath) { throw 'The sample target already exists.' }
  $app=New-Object -ComObject Access.Application
  $app.Visible=$false; $app.AutomationSecurity=3
  $app.NewCurrentDatabase([IO.Path]::GetFullPath($OutputPath),12)
  $db=$app.CurrentDb()
  $db.Execute('CREATE TABLE Customers (ID LONG CONSTRAINT pkCustomers PRIMARY KEY, DisplayName TEXT(60) NOT NULL)',128)
  $db.Execute('CREATE TABLE Orders (ID LONG CONSTRAINT pkOrders PRIMARY KEY, CustomerID LONG NOT NULL, Amount CURRENCY)',128)
  $db.Execute('INSERT INTO Customers VALUES (1, ''Synthetic Customer'')',128)
  $db.Execute('INSERT INTO Orders VALUES (1, 1, 42)',128)
  $db.Execute('ALTER TABLE Orders ADD CONSTRAINT fkCustomers FOREIGN KEY (CustomerID) REFERENCES Customers (ID)',128)
  [void]$db.CreateQueryDef('OrdersOverview','SELECT Orders.ID, Customers.DisplayName, Orders.Amount FROM Customers INNER JOIN Orders ON Customers.ID = Orders.CustomerID')
  $form=$app.CreateForm(); $formName=[string]$form.Name
  $form.RecordSource='OrdersOverview'; $form.Caption='Synthetic orders'
  $form.HasModule=$true
  $formModule=$form.Module
  $formModule.InsertText('Private Sub Form_Load()' + "`r`n" + '  DoCmd.OpenQuery "OrdersOverview"' + "`r`n" + '  CurrentDb.Execute "UPDATE Orders SET Amount = 42"' + "`r`n" + 'End Sub')
  $app.DoCmd.Save(2,$formName); $app.DoCmd.Close(2,$formName,1); $app.DoCmd.Rename('OrdersEntry',2,$formName)
  # Deliberately inaccessible link: extraction must not try to connect or count rows.
  $externalPath=Join-Path (Split-Path ([IO.Path]::GetFullPath($OutputPath)) -Parent) ('synthetic-unavailable-'+[Guid]::NewGuid().ToString('N')+'.accdb')
  $externalDb=$app.DBEngine.CreateDatabase($externalPath,';LANGID=0x0409;CP=1252;COUNTRY=0',128)
  $externalDb.Execute('CREATE TABLE Missing (ID LONG)',128); $externalDb.Close(); [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($externalDb)
  $linked=$db.CreateTableDef('UnavailableExternal'); $linked.Connect=";DATABASE=$externalPath"; $linked.SourceTableName='Missing'; $db.TableDefs.Append($linked)
  Write-Output 'SYNTHETIC_SAMPLE_CREATED'
} finally {
  if ($db) { $db.Close(); [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($db) }
  if ($app) { try { $app.CloseCurrentDatabase() } catch {}; $app.Quit(2); [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($app) }
  [GC]::Collect(); [GC]::WaitForPendingFinalizers()
  if ($externalPath -and (Test-Path -LiteralPath $externalPath)) { Remove-Item -LiteralPath $externalPath -Force }
}
