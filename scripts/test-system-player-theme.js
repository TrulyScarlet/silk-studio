// Evaluate on a fresh Vite /?preview=1 browser tab. Uses fake IPC only.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) throw Error('Preview only');
  const assert = (ok, why) => { if (!ok) throw Error(why); };
  const delay = (ms=100) => new Promise(r=>setTimeout(r,ms));
  const until = async (test) => { for(let i=0;i<200;i++){if(test())return;await delay(30);}throw Error('Timed out'); };
  const button = text => [...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text);
  const click = async el => { assert(el,'Missing control');el.click();await delay(); };
  const key = async (code, shiftKey=false) => { (document.activeElement??document.body).dispatchEvent(new KeyboardEvent('keydown',{code,shiftKey,bubbles:true}));await delay(); };
  const input = async (el,value) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,value);el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));await delay(); };
  let saved, fullscreen=false;
  window.__TAURI_INTERNALS__ = {
    convertFileSrc:()=> 'http://127.0.0.1:7991/clip.mp4',
    invoke:async(cmd,args)=>{
      if(cmd==='configure_settings'){saved=structuredClone(args.settings);return;}
      if(cmd==='app_window_is_fullscreen')return fullscreen;
      if(cmd==='app_window_set_fullscreen'){fullscreen=args.fullscreen;return;}
      if(cmd==='get_clip_frame_rate')return 60;
      if(cmd==='get_clip_keyframes')return [0,2,4];
      if(cmd==='get_clip_audio_tracks')return [{trackId:1,name:'Game',channels:2,sampleRate:48000}];
      if(cmd==='update_theme_icon')return;
      throw Error('Preview fixture');
    }
  };
  const openSettings = () => click(document.querySelector('[aria-label="Open Settings dialog"]'));
  await openSettings();await click(button('Themes'));
  assert(!document.body.textContent.includes('System Behavior'),'System separated from Themes');
  const original=document.documentElement.style.getPropertyValue('--accent');
  await input(document.querySelector('[aria-label="Main color"]'),'#123456');
  assert(document.documentElement.style.getPropertyValue('--accent')===original,'Draft colors must not leak');
  await click(document.querySelector('.settings-modal-close-btn'));
  await openSettings();await click(button('Themes'));
  assert(document.querySelector('[aria-label="Main color"]').value!=='#123456','Discard custom colors');
  await input(document.querySelector('[aria-label="Main color"]'),'#abcdef');
  await input(document.querySelector('[aria-label="Background color"]'),'#111122');
  await input(document.querySelector('[aria-label="Capture color"]'),'#ff9900');
  await input(document.querySelector('[aria-label="Replay buffer color"]'),'#3377ee');
  await click(button('System'));
  const auto=[...document.querySelectorAll('.check-row')].find(e=>e.textContent.includes('Open clips in fullscreen')).querySelector('input');
  if(!auto.checked)await click(auto);
  document.querySelector('form').requestSubmit();await until(()=>saved?.application.openClipsFullscreen);
  assert(saved.application.colors.main==='#abcdef','Color persisted');
  await until(()=>document.documentElement.style.getPropertyValue('--accent')==='#abcdef');
  assert(document.documentElement.style.getPropertyValue('--signal')==='#3377ee','Buffer color applied');
  assert(document.documentElement.style.getPropertyValue('--capture-color')==='#ff9900','Capture color applied separately');
  await click(document.querySelector('.settings-modal-close-btn'));
  await click(document.querySelector('[aria-label^="Preview "]'));
  await until(()=>fullscreen && document.querySelector('video')?.readyState>=1);
  const chrome=document.querySelector('.clip-playback-chrome');
  const motion=document.createElement('style');motion.textContent='.clip-playback-chrome,.clip-fullscreen-topbar{transition:none!important}';document.head.append(motion);
  document.querySelector('.clip-video-stage').dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:innerWidth/2,clientY:innerHeight/2}));await delay();
  assert(getComputedStyle(chrome).opacity==='0','Center movement must not show edge controls');
  const video=document.querySelector('video');video.muted=true;
  await key('Space');await until(()=>!video.paused && document.querySelector('.clip-transport-controls [aria-label="Pause"]'));
  await key('KeyK');assert(video.paused,'K pauses');
  await key('Digit5');await until(()=>Math.abs(video.currentTime-video.duration/2)<0.2);
  const before=video.currentTime;await key('Period');await until(()=>video.currentTime>before);
  assert(video.paused && Math.abs(video.currentTime-before-1/60)<0.005,'Period steps frame while paused');
  await key('Comma');await until(()=>Math.abs(video.currentTime-before)<0.005);
  await key('Period',true);assert(video.playbackRate===1.25,'Shift period changes speed');
  await key('KeyF');assert(!fullscreen,'F exits fullscreen without closing');
  const picker=document.querySelector('[aria-label="Custom bookmark color"]');await input(picker,'#17aaee');
  await click([...document.querySelectorAll('button')].find(b=>b.textContent.includes('+ Add Bookmark')));
  const newMarker=[...document.querySelectorAll('.vod-marker-color')].find(el=>el.value==='#17aaee');
  assert(newMarker,'Bookmark custom color saved');
  await input(newMarker,'#aa3377');
  assert(newMarker.value==='#aa3377','Existing bookmark recolored');
  await key('KeyF');assert(fullscreen,'F reenters');
  await key('Escape');assert(!document.querySelector('.clip-player-root')&&!fullscreen,'Esc closes clip and fullscreen');
  motion.remove();
  delete window.__TAURI_INTERNALS__;
  return {passed:true,colors:saved.application.colors,autoFullscreen:saved.application.openClipsFullscreen};
})()
