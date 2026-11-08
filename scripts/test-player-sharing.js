// Run on an isolated /?preview=1 browser. Native calls are recorded, never executed.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) throw Error('Use a fresh browser preview');
  const delay = (ms=80) => new Promise(r=>setTimeout(r,ms));
  const assert = (ok, message) => { if(!ok) throw Error(message); };
  const until = async test => { for(let i=0;i<150;i++){if(test())return;await delay(30);}throw Error('Timed out'); };
  const button = name => [...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name || b.getAttribute('aria-label')===name);
  const click = async el => { assert(el,'Missing control');el.click();await delay(); };
  const calls=[]; let fullscreen=false, stage='encoding', nextId=0, rejectExport=false;
  window.__TAURI_INTERNALS__ = {
    convertFileSrc:()=> 'http://127.0.0.1:7991/clip.mp4',
    invoke:async(command,args)=>{
      calls.push({command,args});
      if(command==='app_window_is_fullscreen')return fullscreen;
      if(command==='app_window_set_fullscreen'){fullscreen=args.fullscreen;return;}
      if(command==='get_clip_frame_rate')return 60;
      if(command==='get_clip_keyframes')return [0,2,4];
      if(command==='get_clip_audio_tracks')return [{trackId:1,name:'Game',channels:2,sampleRate:48000}];
      if(command==='prepare_clip_share'){
        if(args.targetMb===null)return {jobId:null,status:'ready',stage:'ready',sizeBytes:130000000,error:null};
        if(rejectExport)throw Error('Sharing tools unavailable for test');
        return {jobId:++nextId,status:'preparing',stage:'queued',sizeBytes:null,error:null};
      }
      if(command==='clip_share_status')return {jobId:args.jobId,status:stage==='ready'?'ready':'preparing',stage,sizeBytes:stage==='ready'?9700000:null,error:null};
      if(['cancel_clip_share','drag_clip_share','reveal_clip_share'].includes(command))return;
      throw Error('Preview fixture');
    }
  };
  try {
    assert(!document.querySelector('main > .activity-panel'),'Activity removed from library');
    assert(Math.abs(innerHeight-document.querySelector('.hero-library-section').getBoundingClientRect().bottom-12)<2,'Library ends near bottom');
    assert(getComputedStyle(document.querySelector('.state-ready')).color==='rgb(239, 239, 237)','Default capture neutral');
    assert(getComputedStyle(document.documentElement).getPropertyValue('--signal').trim()==='#efefed','Default buffer neutral');
    await click(button('Open Settings dialog')); await click(button('Diagnostics'));
    assert(document.querySelector('.diagnostic-activity'),'Log available in Diagnostics');
    await click(document.querySelector('.settings-modal-close-btn'));
    await click(document.querySelector('.clip-card-thumb-container'));
    await until(()=>document.querySelector('video')?.readyState>=1);
    assert(button('Next clip')&&!button('Previous clip'),'Standard navigation boundary');
    const first=document.querySelector('.clip-modal-title-group h2').textContent;
    await click(button('Next clip')); assert(button('Previous clip'),'Standard previous arrow');
    await click(button('Previous clip')); assert(document.querySelector('.clip-modal-title-group h2').textContent===first,'Standard previous selects clip');
    await click(button('Theater Mode')); await click(button('Next clip')); assert(button('Previous clip'),'Theater navigation');
    await click(button('Fullscreen'));
    assert(document.querySelectorAll('[aria-label="Exit Fullscreen"]').length===1&&!document.querySelector('.clip-fullscreen-topbar'),'One fullscreen exit');
    const transport=document.querySelector('.clip-transport-controls');
    assert([...transport.children].map(b=>b.textContent.trim()).join(',')==='-5s,Play,+5s','Transport order');
    const r=transport.getBoundingClientRect();
    assert(Math.abs(r.x+r.width/2-innerWidth/2)<2,'Transport centered');
    assert(document.querySelector('.rate-selector-group').getBoundingClientRect().left<30,'Speeds far left');
    await click(button('Exit Fullscreen')); await click(button('Share'));
    await until(()=>document.querySelector('.share-drop-zone.is-ready'));
    await until(()=>document.querySelector('.share-clip-preview video')?.readyState>=2);
    assert(document.querySelector('.share-clip-preview video').paused,'Ready thumbnail stays paused');
    assert(getComputedStyle(document.querySelector('.share-clip-preview')).pointerEvents==='none','Thumbnail leaves drag gesture with parent');
    await click(button('Original')); assert(document.querySelector('.share-drop-zone.is-ready'),'Selecting current preset keeps file');
    document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{code:'KeyF',bubbles:true})); await delay();
    assert(!fullscreen,'Player shortcut ignored behind share dialog');
    await click(button('10 MB')); assert(!document.querySelector('.share-drop-zone.is-ready'),'Changed size invalidates ready file');
    await click(button('Prepare clip')); await until(()=>document.querySelector('progress'));
    assert([...document.querySelectorAll('.share-presets button')].every(b=>b.disabled),'No competing preparations');
    stage='ready'; await until(()=>document.querySelector('.share-drop-zone.is-ready'));
    assert(document.querySelector('.share-drop-zone').textContent.includes('9.70 MB'),'Actual size shown');
    await until(()=>document.querySelector('.share-clip-preview video')?.readyState>=2);
    const drop=document.querySelector('.share-clip-preview');
    drop.dispatchEvent(new PointerEvent('pointerdown',{button:0,buttons:1,clientX:200,clientY:200,bubbles:true}));
    drop.dispatchEvent(new PointerEvent('pointermove',{buttons:1,clientX:220,clientY:210,bubbles:true}));await delay();
    assert(calls.some(c=>c.command==='drag_clip_share'&&c.args.jobId===1),'Prepared artifact used for drag');
    await click(button('Show file in folder'));assert(calls.some(c=>c.command==='reveal_clip_share'&&c.args.jobId===1),'Reveal uses prepared artifact');
    await click(button('50 MB'));stage='encoding';await click(button('Prepare clip'));await click(button('Cancel'));
    assert(calls.some(c=>c.command==='cancel_clip_share'&&c.args.jobId===2),'Close cancels active export');
    assert(document.querySelector('.clip-modal-container'),'Share close retains player');
    await click(button('Share'));await click(button('100 MB'));rejectExport=true;await click(button('Prepare clip'));
    await until(()=>document.querySelector('.share-error'));
    assert(!document.querySelector('.share-drop-zone.is-ready'),'Failed exports cannot be dragged');
    assert(!document.querySelector('.share-clip-preview'),'Failed exports have no ready thumbnail');
    assert(!button('Prepare clip').disabled,'Failed preparation can retry');
    document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{code:'Escape',key:'Escape',bubbles:true}));await delay();
    assert(!document.querySelector('.clip-share-dialog')&&document.querySelector('.clip-modal-container'),'Escape closes only share');
    await click(button('Close clip player'));
    const thumb=document.querySelector('.clip-card-thumb-container');
    thumb.dispatchEvent(new PointerEvent('pointerdown',{button:0,buttons:1,clientX:200,clientY:200,bubbles:true}));
    thumb.dispatchEvent(new PointerEvent('pointermove',{buttons:1,clientX:210,clientY:210,bubbles:true}));await delay();thumb.click();await delay();
    assert(calls.some(c=>c.command==='drag_clip_share'&&c.args.jobId===null),'Library drag uses original');
    assert(!document.querySelector('.clip-modal-container'),'Drag does not open player');
    await click(button('List'));assert(document.querySelector('.clip-row .clip-share-button'),'List share action');await click(button('Grid'));
    return {passed:true,calls:calls.filter(c=>c.command.includes('share')),viewport:[innerWidth,innerHeight]};
  } finally { document.querySelector('[aria-label="Close share dialog"]')?.click();document.querySelector('[aria-label="Close clip player"]')?.click();delete window.__TAURI_INTERNALS__; }
})()
