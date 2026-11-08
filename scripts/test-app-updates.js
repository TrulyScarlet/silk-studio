// Run only in an isolated browser preview. No native installer is invoked.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) throw Error('Use a fresh isolated preview');
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const assert = (value, message) => { if (!value) throw Error(message); };
  const until = async test => { for (let i=0;i<100;i++) { if(test())return; await delay(60); } throw Error('Update UI condition timed out'); };
  const button = (root, label) => [...root.querySelectorAll('button')].find(b => b.textContent.trim()===label);
  let status = {currentVersion:'0.1.0',version:null,notes:null,phase:'idle',downloaded:0,total:null,error:null};
  let next='current', installs=0, finishInstall;
  const calls=[];
  window.__TAURI_INTERNALS__ = {invoke: async (command,args) => {
    if (command==='get_update_status')return {...status};
    if (!['check_for_updates','install_update'].includes(command))throw Error('Isolated fixture');
    calls.push({command,args});
    if (command==='check_for_updates') {
      status={...status,phase:'checking',error:null}; await delay(120);
      status={...status,phase:next,version:next==='available'?'0.2.0':null,notes:next==='available'?'Capture improvements and clip sharing.':null};
    } else {
      installs++;
      status={...status,phase:'downloading',downloaded:250000,total:1000000,error:null};
      await new Promise((resolve,reject)=>{finishInstall={resolve,reject};});
    }
  }};
  const ReactModule = await import('/node_modules/.vite/deps/react.js');
  const React = ReactModule.default ?? ReactModule;
  const ReactDOMModule = await import('/node_modules/.vite/deps/react-dom_client.js');
  const {createRoot} = ReactDOMModule.default ?? ReactDOMModule;
  const {TitleBarUpdate,UpdateSettings} = await import('/src/AppUpdates.tsx');
  const titleMount=document.createElement('div'); document.querySelector('.silk-titlebar').append(titleMount);
  const controlsMount=document.createElement('div'); controlsMount.className='update-test-controls';
  document.querySelector('main').append(controlsMount);
  const titleRoot=createRoot(titleMount), controlsRoot=createRoot(controlsMount);
  titleRoot.render(React.createElement(TitleBarUpdate)); controlsRoot.render(React.createElement(UpdateSettings));
  await until(()=>button(controlsMount,'Check for updates'));
  assert(!titleMount.querySelector('.titlebar-update-pill'),'No fabricated update notification');
  button(controlsMount,'Check for updates').click();
  await until(()=>controlsMount.textContent.includes("You're up to date."));
  assert(!titleMount.querySelector('.titlebar-update-pill'),'No update pill for current version');
  next='available'; button(controlsMount,'Check for updates').click();
  await until(()=>titleMount.querySelector('.titlebar-update-pill'));
  titleMount.querySelector('.titlebar-update-pill').click();
  await until(()=>titleMount.querySelector('[role="dialog"]'));
  assert(titleMount.textContent.includes('0.2.0'),'Real available version shown');
  assert(titleMount.textContent.includes('Save any replay'),'Unsaved replay notice appears before install');
  button(titleMount,'Update & restart').click();
  await until(()=>titleMount.textContent.includes('25%'));
  assert(titleMount.querySelector('.titlebar-update-action').disabled,'Install disabled during download');
  assert(installs===1,'Exactly one installation requested');
  status={...status,downloaded:1000000,phase:'verifying'};
  await until(()=>titleMount.textContent.includes('Verifying update'));
  status={...status,phase:'error',error:'Update verification failed. The current version is unchanged.'};
  finishInstall.reject(Error(status.error));
  await until(()=>button(titleMount,'Retry update & restart'));
  assert(titleMount.querySelector('[role="alert"]'),'Verification error is visible and retryable');
  button(titleMount,'Retry update & restart').click();
  await until(()=>installs===2);
  status={...status,phase:'installing'}; finishInstall.resolve();
  await until(()=>titleMount.textContent.includes('Silk will restart'));
  assert(titleMount.querySelector('.titlebar-update-action').disabled,'No premature successful restart claim');
  status={...status,phase:'available',downloaded:0,total:null,error:null};
  await until(()=>button(titleMount,'Update & restart'));
  titleMount.querySelector('.titlebar-update-pill').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
  await until(()=>!titleMount.querySelector('[role="dialog"]'));
  titleMount.querySelector('.titlebar-update-pill').click();
  controlsRoot.unmount(); controlsMount.remove();
  window.__silkUpdateFixture={status,calls};
  return {passed:true,installs,calls: calls.map(call=>call.command)};
})()
