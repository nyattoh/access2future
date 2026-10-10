$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
function Get-SampleHash([string]$path) {
  $stream=[IO.File]::OpenRead($path)
  $sha=[Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','') }
  finally { $stream.Dispose(); $sha.Dispose() }
}
$directory=Join-Path ([IO.Path]::GetTempPath()) ('access-native-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
try {
  $sample=Join-Path $directory 'synthetic.accdb'
  & (Join-Path $root 'scripts/create-sample.ps1') -OutputPath $sample
  $before=Get-SampleHash $sample
  $env:ACCESS_SYNTHETIC_SAMPLE=$sample
  Push-Location $root
  try {
    @'
import assert from 'node:assert/strict';
import {importAccess,getAccessCapabilities} from './src/access-import.mjs';
assert.equal((await getAccessCapabilities()).available,true);
const progress=[];
const i=await importAccess(process.env.ACCESS_SYNTHETIC_SAMPLE,{onProgress:s=>progress.push(s)});
assert.equal(i.schemaVersion,1); assert.equal(i.source.kind,'access');
const form=i.assets.find(a=>a.id==='form:OrdersEntry'); assert.ok(form);
assert.ok(form.dependsOn.includes('query:OrdersOverview'));
assert.equal(form.hasCodeModule,true); assert.equal(form.moduleType,'class'); assert.ok(form.procedureNames.includes('Form_Load'));
assert.ok(form.dependsOn.includes('table:Orders'));
const query=i.assets.find(a=>a.id==='query:OrdersOverview'); assert.ok(query.dependsOn.includes('table:Customers'));
assert.ok(i.relations.some(r=>r.from==='table:Orders'&&r.to==='table:Customers'&&r.enforced));
assert.equal(i.assets.find(a=>a.id==='table:UnavailableExternal').status,'unsupported');
assert.ok(i.assets.every(a=>a.rowCount===undefined)); assert.ok(!JSON.stringify(i).includes('DATABASE=')); assert.ok(!/[A-Z]:\\\\/i.test(JSON.stringify(i)));
assert.ok(progress.length>=2, 'at least preparing and finishing snapshots');
assert.deepEqual(progress[0],{phase:'preparing',completed:0,total:null});
for (const s of progress) {
  assert.deepEqual(Object.keys(s).sort(),['completed','phase','total']);
  assert.ok(['preparing','enumerating','analysing','finishing'].includes(s.phase));
  assert.ok(Number.isSafeInteger(s.completed)&&s.completed>=0);
  if (s.total!==null) { assert.equal(s.total,i.assets.length); assert.ok(s.completed<=s.total); }
}
for (let k=1;k<progress.length;k+=1) assert.ok(progress[k].completed>=progress[k-1].completed);
const last=progress[progress.length-1];
assert.equal(last.phase,'finishing'); assert.equal(last.completed,i.assets.length); assert.equal(last.total,i.assets.length);
console.log('NATIVE_SYNTHETIC_PASS',JSON.stringify({assets:i.assets.length,relations:i.relations.length,limitations:i.limitations.length,progressEvents:progress.length,last:`${last.completed}/${last.total}`}));
'@ | node --input-type=module
    if ($LASTEXITCODE -ne 0) { throw 'Native assertions failed.' }
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargoCommand) {
      $cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
      if (Test-Path -LiteralPath $cargoPath) { $cargoCommand = Get-Item -LiteralPath $cargoPath }
    }
    if (-not $cargoCommand) {
      Write-Output 'RUST_NATIVE_NOT_RUN no Rust toolchain found'
    } else {
      $env:ACCESS2FUTURE_SYNTHETIC_SAMPLE = $sample
      $cargoExecutable = if ($cargoCommand.Source) { $cargoCommand.Source } else { $cargoCommand.FullName }
      & $cargoExecutable test --manifest-path (Join-Path $root 'src-tauri\Cargo.toml') rust_access_native_smoke -- --ignored --nocapture
      if ($LASTEXITCODE -ne 0) { throw 'Rust native assertions failed.' }
    }
  } finally { Pop-Location }
  if ((Get-SampleHash $sample) -ne $before) { throw 'Source changed.' }
  Write-Output 'SOURCE_HASH_UNCHANGED'
} finally {
  Remove-Item Env:ACCESS_SYNTHETIC_SAMPLE -ErrorAction SilentlyContinue
  Remove-Item Env:ACCESS2FUTURE_SYNTHETIC_SAMPLE -ErrorAction SilentlyContinue
  $resolved=[IO.Path]::GetFullPath($directory); $temp=[IO.Path]::GetFullPath([IO.Path]::GetTempPath())
  if ($resolved.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $directory -Recurse -Force }
}
