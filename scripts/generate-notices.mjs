// Generate redistribution notices from the exact lockfile packages and their
// included license texts. Does not download or copy implementation sources.
import { execFileSync } from 'node:child_process';
import { readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

const root=resolve(import.meta.dirname,'..');
const metadata=JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1','--filter-platform','x86_64-pc-windows-msvc'],{cwd:root,maxBuffer:32*1024*1024,encoding:'utf8'}));
const resolved=new Set(metadata.resolve.nodes.map(node=>node.id));
const sections=['Silk third-party notices\nGenerated from Cargo.lock and package-lock.json.\nIncludes build tools as well as distributed dependencies.\n'];
const missing=[];
const overrides=JSON.parse(await readFile(join(root,'licenses/overrides/sources.json'),'utf8'));
async function notices(name,version,license,location,source) {
  sections.push(`\n${'='.repeat(72)}\n${name} ${version}\nLicense: ${license}\nSource: ${source}\n`);
  const files=(await readdir(location,{withFileTypes:true})).filter(entry=>entry.isFile()&&/^(licen[sc]e|copying|notice|copyright)(\.|-|_|$)/i.test(entry.name));
  if (!files.length) {
    const id=`${name}-${version}`;
    if(overrides[id]) sections.push(`License text source: ${overrides[id]}\n${await readFile(join(root,'licenses/overrides',id+'.txt'),'utf8')}`);
    else missing.push(`${name} ${version}`);
  }
  for(const file of files.sort((a,b)=>a.name.localeCompare(b.name))) sections.push(`${file.name}\n${await readFile(join(location,file.name),'utf8')}\n`);
}
for(const pkg of metadata.packages.filter(pkg=>pkg.source&&resolved.has(pkg.id)).sort((a,b)=>a.name.localeCompare(b.name))) {
  await notices(pkg.name,pkg.version,pkg.license,dirname(pkg.manifest_path),`https://crates.io/crates/${pkg.name}/${pkg.version}`);
}
const frontend=join(root,'apps/desktop');
const lock=JSON.parse(await readFile(join(frontend,'package-lock.json'),'utf8'));
for(const [path,info] of Object.entries(lock.packages).filter(([path,info])=>path&&!info.dev&&!info.devOptional).sort()) {
  const location=join(frontend,path);
  let pkg;
  try {pkg=JSON.parse(await readFile(join(location,'package.json'),'utf8'));} catch {continue;} // Other OS optional dependencies.
  await notices(pkg.name,info.version,pkg.license??info.license??'See included notice',location,`https://www.npmjs.com/package/${pkg.name}/v/${info.version}`);
}
if(missing.length) {
  console.error(`Packages without an included top-level notice: ${missing.join(', ')}`);
  process.exitCode=1;
}
await mkdir(join(root,'licenses'),{recursive:true});
await writeFile(join(root,'licenses/THIRD_PARTY_NOTICES.txt'),sections.join('\n').replace(/[ \t]+(?=\r?$)/gm,'').trimEnd()+'\n');
console.log(`Wrote ${sections.length} notice sections; ${missing.length} missing texts.`);
