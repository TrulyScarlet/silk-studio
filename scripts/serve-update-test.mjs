// Loopback-only feed for the opt-in updater-test-server build. Never publish.
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const installer=resolve(process.argv[2]??'target/release/bundle/nsis/Silk_0.1.0_x64-setup.exe');
const modeFile=resolve('target/updater-test-mode.txt');
http.createServer(async (request,response)=>{
  try {
    const mode=(await readFile(modeFile,'utf8')).trim();
    if(mode==='offline'){response.writeHead(503).end();return;}
    if(request.url==='/latest.json') {
      const manifest={version:mode==='wrong-version'?'0.1.1':'0.1.0',notes:'Local upgrade validation.',platforms:{'windows-x86_64':{
        url:'http://127.0.0.1:7992/installer.exe',signature:(await readFile(installer+'.sig','utf8')).trim()
      }}};
      response.writeHead(200,{'Content-Type':'application/json'}).end(JSON.stringify(manifest));
    } else if(request.url==='/installer.exe') {
      const bytes=await readFile(installer);
      if(mode==='corrupt')bytes[bytes.length-1]^=1;
      response.writeHead(200,{'Content-Type':'application/octet-stream','Content-Length':bytes.length}).end(bytes);
    } else response.writeHead(404).end();
  }catch{response.writeHead(500).end();}
}).listen(7992,'127.0.0.1',()=>console.log('Local updater test feed listening on 127.0.0.1:7992'));
