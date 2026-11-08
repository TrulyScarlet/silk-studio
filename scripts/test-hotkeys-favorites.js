// Run via preview_evaluate on a fresh Vite /?preview=1 tab.
// Uses an isolated IPC fixture and the read-only local video test server on :7991.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) throw Error('Use a fresh browser preview.');
  const assert = (value, message) => { if (!value) throw Error(message); };
  const wait = (ms=100) => new Promise(resolve => setTimeout(resolve, ms));
  const until = async (test) => { for(let i=0;i<150;i++){ if(test()) return; await wait(40); } throw Error('Timed out waiting for UI state'); };
  const button = text => [...document.querySelectorAll('button')].find(b => b.textContent.trim() === text);
  const click = async element => { assert(element, 'Missing control'); element.click(); await wait(); };
  const press = async (code, mods={}, target=document.body) => {
    target.dispatchEvent(new KeyboardEvent('keydown', {code, key:code==='Escape'?'Escape':code, ...mods, bubbles:true, cancelable:true})); await wait();
  };
  const input = async (element,value) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(element,value);
    element.dispatchEvent(new Event('input',{bubbles:true})); await wait();
  };
  const shortcut = label => document.querySelector(`[aria-label^="${label} hotkey:"]`);
  const record = async (label, code, mods={}) => { await click(shortcut(label)); await press(code,mods,shortcut(label)); };
  const rowButton = (label, text) => [...shortcut(label).closest('.hotkey-recorder-field').querySelectorAll('button')].find(b=>b.textContent.trim()===text);
  const openSettings = () => click(document.querySelector('[aria-label="Open Settings dialog"]'));
  const fixture = [
    {path:'test/alpha.mp4',name:'Alpha',gameName:'Valorant',createdAtUnixMs:3000,durationMs:44900,sizeBytes:100,missing:false,protected:true},
    {path:'test/bravo.mp4',name:'Bravo',gameName:'Valorant',createdAtUnixMs:2000,durationMs:44900,sizeBytes:100,missing:false,protected:false},
    {path:'test/charlie.mp4',name:'Charlie',gameName:'Apex Legends',createdAtUnixMs:1000,durationMs:44900,sizeBytes:100,missing:false,protected:false},
  ];
  let saved, fullscreen=false;
  window.__TAURI_INTERNALS__ = {
    convertFileSrc:()=> 'http://127.0.0.1:7991/clip.mp4',
    invoke:async (command,args)=>{
      if(command==='configure_settings') {saved=structuredClone(args.settings);return;}
      if(command==='list_clips') return structuredClone(fixture);
      if(command==='set_clip_protected') {const clip=fixture.find(c=>c.path===args.path);clip.protected=args.protected;return structuredClone(clip);}
      if(command==='get_storage_summary')return {outputDirectory:'Preview',totalBytes:300,quotaBytes:null,protectedBytes:fixture.filter(c=>c.protected).length*100,eligibleBytes:0,automaticDeletionEnabled:false,deletedCount:0,deletedBytes:0};
      if(command==='app_window_is_fullscreen')return fullscreen;
      if(command==='app_window_set_fullscreen'){fullscreen=args.fullscreen;return;}
      if(command==='get_clip_frame_rate')return 60;
      if(command==='get_clip_keyframes')return [0,2,4];
      if(command==='get_clip_audio_tracks')return [{trackId:1,name:'Game',channels:2,sampleRate:48000}];
      if(command==='update_theme_icon')return;
      throw Error('Isolated preview fixture: '+command);
    }
  };
  try {
    await click(document.querySelector('.refresh-library-btn'));
    assert(document.querySelectorAll('.clip-card').length===3,'Fixture library loaded');
    await click(document.querySelector('.library-collection-filter button:last-child'));
    assert(document.querySelectorAll('.clip-card').length===1,'Favorites grid filters by star');
    await click(document.querySelector('[title="Switch to List View"]'));
    assert(document.querySelectorAll('[aria-label="Remove from favorites"]').length===1 && !document.querySelector('[aria-label="Add to favorites"]'),'List uses same favorites filter');
    await click([...document.querySelectorAll('.game-chip')].find(b=>b.textContent.startsWith('Apex Legends')));
    assert(document.querySelector('.library-empty').textContent.includes('No favorites'),'Game filter combines with Favorites');
    await click([...document.querySelectorAll('.game-chip')].find(b=>b.textContent.startsWith('All Games')));
    await click(document.querySelector('[aria-label="Remove from favorites"]'));
    assert(document.querySelector('.library-empty').textContent.includes('No favorites'),'Unfavoriting updates active filter');
    await click(button('All clips'));
    await click(document.querySelector('[aria-label="Add to favorites"]'));
    await click(document.querySelector('.library-collection-filter button:last-child'));
    await input(document.querySelector('.library-search-input'),'Bravo');
    assert(document.querySelector('.library-empty'),'Search combines with Favorites');
    await click(document.querySelector('[aria-label="Clear search"]'));
    await click(button('All clips'));
    await click(document.querySelector('[title="Switch to Grid View"]'));

    await openSettings();
    const tabs=[...document.querySelectorAll('.settings-nav-sidebar button')].map(b=>b.textContent.trim());
    assert(tabs.includes('Capture')&&tabs.includes('Hotkeys')&&!tabs.includes('Capture & Hotkeys'),'Separate categories');
    assert(tabs.indexOf('System')===tabs.indexOf('Diagnostics')-1,'System before Diagnostics');
    await click(button('System'));
    assert(document.querySelectorAll('.settings-tab-pane .settings-group-card').length===1,'One joined System panel');
    assert(document.querySelector('.system-settings-card .check-row').textContent.startsWith('Open clips in fullscreen'),'Fullscreen is first');
    await click(button('Hotkeys'));
    assert(document.querySelectorAll('.hotkey-record-btn').length===31,'All global and local shortcuts exposed');
    await record('Toggle fullscreen','F9');
    await click(document.querySelector('.settings-modal-close-btn'));
    await openSettings();await click(button('Hotkeys'));
    assert(shortcut('Toggle fullscreen').textContent.trim()==='F','Unsaved binding discarded');
    await record('Toggle fullscreen','F9');await record('Mute / unmute','F9');
    assert(document.querySelector('.settings-save-btn').disabled && document.querySelector('.shortcut-conflicts'),'Conflict blocks save');
    await click(rowButton('Mute / unmute','Reset default'));
    assert(!document.querySelector('.settings-save-btn').disabled,'Reset resolves conflict');
    await record('Toggle fullscreen','KeyF',{ctrlKey:true});
    await record('Play / pause','KeyP');
    await click(rowButton('Play / pause (alternate)','Clear'));
    await record('Close clip','Escape');
    assert(document.querySelector('.settings-modal-container'),'Escape can be recorded without closing Settings');
    await record('Close clip','KeyX');
    await record('Next frame','BracketRight');await record('Previous frame','BracketLeft');
    await record('Jump to 50%','Home');
    await record('Toggle theater mode','KeyY');
    await record('Mute / unmute','KeyM',{ctrlKey:true});
    await record('Forward 5 seconds','ArrowRight',{altKey:true});
    await record('Increase playback speed','Equal');await record('Decrease playback speed','Minus');
    await record('Next clip','KeyN',{shiftKey:true});await record('Previous clip','KeyP',{shiftKey:true});
    await record('Add bookmark','KeyB',{ctrlKey:true});
    await record('Open settings','KeyS',{ctrlKey:true});
    document.querySelector('form').requestSubmit();await until(()=>saved && !document.querySelector('.settings-save-btn').disabled && !document.querySelector('.settings-dirty-pill'));
    assert(saved.hotkeys.shortcuts.fullscreen==='CTRL+F' && saved.hotkeys.shortcuts.playPauseAlternate===null,'Chords and explicit disables persisted');
    await click(document.querySelector('.settings-modal-close-btn'));
    await press('Comma',{ctrlKey:true});assert(!document.querySelector('.settings-modal-container'),'Old app chord removed');
    await press('KeyS',{ctrlKey:true});assert(document.querySelector('.settings-modal-container'),'Remapped app chord works');
    await click(button('Hotkeys'));
    assert(shortcut('Toggle fullscreen').textContent.replace(/\s/g,'')==='CTRL+F','Saved binding survives reopening');
    await click(document.querySelector('.settings-modal-close-btn'));

    await click(document.querySelector('[aria-label^="Preview "]'));
    await until(()=>document.querySelector('video')?.readyState>=1);
    let video=document.querySelector('video');video.muted=true;
    await press('KeyF');assert(!fullscreen,'Old fullscreen chord removed');
    await press('KeyF',{ctrlKey:true});assert(fullscreen,'Modified fullscreen binding works');
    await press('KeyP');await until(()=>!video.paused);
    await press('KeyK');assert(!video.paused,'Cleared alternate is disabled');
    await press('KeyP');await until(()=>video.paused);
    await press('Home');await until(()=>Math.abs(video.currentTime-video.duration/2)<.2);
    let before=video.currentTime;await press('BracketRight');await until(()=>video.currentTime>before);
    assert(Math.abs(video.currentTime-before-1/60)<.005,'Remapped next frame');
    await press('BracketLeft');await until(()=>Math.abs(video.currentTime-before)<.005);
    await press('ArrowRight',{altKey:true});await until(()=>Math.abs(video.currentTime-before-5)<.2);
    await press('Equal');assert(video.playbackRate===1.25,'Remapped speed up');
    await press('Minus');assert(video.playbackRate===1,'Remapped speed down');
    await press('KeyM',{ctrlKey:true});assert(document.querySelector('[title="Unmute (CTRL+M)"]'),'Remapped mute and tooltip');
    await press('KeyN',{shiftKey:true});await until(()=>document.querySelector('video')!==video);
    assert(fullscreen && document.querySelector('.clip-modal-title-group h2').textContent.includes('Bravo'),'Next clip preserves fullscreen');
    await press('KeyP',{shiftKey:true});await until(()=>document.querySelector('.clip-modal-title-group h2').textContent.includes('Alpha'));
    await press('KeyF',{ctrlKey:true});assert(!fullscreen,'Remapped fullscreen exits');
    await press('KeyY');assert(document.querySelector('.is-theater-layout'),'Remapped theater');
    const marker=document.querySelector('.vod-note-input');
    await input(marker,'Shortcut fixture');await press('KeyX',{},marker);
    assert(document.querySelector('.clip-player-root'),'Typing does not trigger close shortcut');
    await press('KeyB',{ctrlKey:true});assert(document.querySelector('.vod-marker-info').textContent.includes('Shortcut fixture'),'Remapped bookmark action');
    await press('Escape');assert(document.querySelector('.clip-player-root'),'Old close binding removed');
    await press('KeyX');assert(!document.querySelector('.clip-player-root'),'Remapped close works');
    const {resolveShortcuts,shortcutAction,shortcutConflicts}=await import('/src/shortcuts.ts');
    const legacy={saveReplay:'F',startCapture:null,stopCapture:null};
    assert(resolveShortcuts(undefined,legacy).fullscreen===null && shortcutConflicts(legacy).length===0,'Legacy global binding keeps precedence');
    assert(shortcutAction({code:'KeyF',ctrlKey:false,altKey:false,shiftKey:true,metaKey:false},resolveShortcuts())===undefined,'Modifiers must match exactly');
    return {passed:true,shortcutControls:31,favorites:'grid, list, search, game, add/remove',persistedOverrides:saved.hotkeys.shortcuts};
  } finally {delete window.__TAURI_INTERNALS__;}
})()
