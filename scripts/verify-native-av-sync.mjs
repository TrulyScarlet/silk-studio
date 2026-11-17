// Opt-in owned flashing-window / loopback acceptance; no game or user config edits.
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
const root = resolve(import.meta.dirname, '..');
const work = mkdtempSync(join(process.env.LOCALAPPDATA, 'Temp/opencode/silk-av-'));
const ffmpeg = join(process.env.LOCALAPPDATA, 'Silk/tools/ffmpeg/bin/ffmpeg.exe');
const ffprobe = join(process.env.LOCALAPPDATA, 'Silk/tools/ffmpeg/bin/ffprobe.exe');
if (createHash('sha256').update(readFileSync(ffmpeg)).digest('hex').toUpperCase() !== '57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A') throw Error('Unapproved decoder');
function run(exe, args, options = {}) {
  const r = spawnSync(exe, args, { cwd: root, maxBuffer: 64 * 1024 * 1024, timeout: 120000, ...options });
  if (r.status !== 0) throw Error(`${exe}: ${r.error ?? r.stderr?.toString()}\n${r.stdout?.toString()}`);
  return r.stdout;
}
run('cargo', ['test', '-p', 'silk', '--test', 'native_recovery', '--no-run']);
const data = Buffer.alloc(48000 * 23 * 2);
for (let n = 0; n < data.length / 2; n++) data.writeInt16LE(n % 96000 >= 24000 && n % 96000 < 33600 ? Math.round(12000 * Math.sin(2 * Math.PI * 1000 * n / 48000)) : 0, n * 2);
const header = Buffer.alloc(44);
header.write('RIFF'); header.writeUInt32LE(data.length + 36, 4); header.write('WAVEfmt ', 8);
header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(1, 22);
header.writeUInt32LE(48000, 24); header.writeUInt32LE(96000, 28); header.writeUInt16LE(2, 32); header.writeUInt16LE(16, 34);
header.write('data', 36); header.writeUInt32LE(data.length, 40);
writeFileSync(join(work, 'stimulus.wav'), Buffer.concat([header, data]));
writeFileSync(join(work, 'config.json'), JSON.stringify({settings: {audio: {tracks: [{enabled: true, sourceKind: 'output_loopback', name: 'Acceptance loopback'}]}}}));
const stimulus = spawn('pwsh', ['-NoProfile', '-File', join(root, 'scripts/av-sync-stimulus.ps1'), '-Wave', join(work, 'stimulus.wav')], {stdio: 'inherit'});
let log;
try {
  log = run('cargo', ['test', '-p', 'silk', '--test', 'native_recovery', '--', '--ignored', '--nocapture'], {env: {...process.env, SILK_RECOVERY_CONFIG: join(work, 'config.json')}}).toString();
  writeFileSync(join(work, 'capture.log'), log);
  console.log(log);
} finally { stimulus.kill(); }
const clip = log.match(/Recovery clip: (.+)/)?.[1].trim();
if (!clip) throw Error('No recovery clip');
const frames = JSON.parse(run(ffprobe, ['-v','error','-select_streams','v:0','-show_frames','-show_entries','frame=best_effort_timestamp_time','-of','json',clip]).toString()).frames;
const pixels = run(ffmpeg, ['-v','error','-i',clip,'-an','-vf','crop=128:128:(iw-128)/2:(ih-128)/2,scale=1:1','-fps_mode','passthrough','-pix_fmt','gray','-f','rawvideo','-']);
if (pixels.length !== frames.length) throw Error('Frame count mismatch');
const visual = [];
for (let i=1; i<pixels.length; i++) if (pixels[i]>180 && pixels[i-1]<80) visual.push(Number(frames[i].best_effort_timestamp_time));
const audio = run(ffmpeg, ['-v','error','-i',clip,'-vn','-map','0:a:0','-af','aresample=async=1:first_pts=0','-ac','1','-ar','48000','-f','s16le','-']);
const audible = [];
let active = false;
for (let n=0; n+960<audio.length; n+=960) {
  let sum=0;
  for (let j=0;j<480;j++) sum+=audio.readInt16LE(n+j*2)**2;
  const on=Math.sqrt(sum/480)>1000;
  if(on && !active) audible.push(n/2/48000);
  active=on;
}
const offsets = audible.length ? visual.map(v => (audible.reduce((a,b)=>Math.abs(a-v)<Math.abs(b-v)?a:b)-v)*1000) : [];
const report = {clip, visualEdges:visual, audioEdges:audible, offsetsMs:offsets, limitMs:100};
writeFileSync(join(work,'results.json'),JSON.stringify(report,null,2));
console.log(JSON.stringify(report,null,2), `\nEvidence: ${work}`);
if(offsets.length<3 || offsets.some(v=>Math.abs(v)>100)) throw Error('Decoded A/V markers failed acceptance');
console.log('PASS: decoded native DXGI/WASAPI markers align within 100ms across two video restarts');
