// Browser integration test. Evaluate on /?preview=1; native IPC is deliberately mocked.
(async () => {
  if (!location.search.includes('preview=1') || window.__TAURI_INTERNALS__) throw Error('Fresh preview required');
  const assert=(ok,msg)=>{if(!ok)throw Error(msg)};
  const delay=()=>new Promise(r=>setTimeout(r,100));
  const button=t=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===t);
  const click=async e=>{assert(e,'Missing control');e.click();await delay()};
  let saved, preview;
  window.__TAURI_INTERNALS__={invoke:async (cmd,args)=>{
    if(cmd==='test_clip_sound'){preview=args.volume;return;}
    if(cmd==='configure_settings'){saved=structuredClone(args.settings);return;}
    throw Error('Preview fixture');
  }};
  await click(document.querySelector('[aria-label="Open Settings dialog"]'));
  await click(button('HUD & Sound'));
  const overlay=[...document.querySelectorAll('.check-row')].find(e=>e.textContent.includes('Show capture confirmation HUD')).querySelector('input');
  assert(!overlay.disabled,'HUD must be available');
  if(!overlay.checked)await click(overlay);
  assert(button('Test notification'),'HUD preview must be restored');
  assert(button('Test notification').disabled,'HUD preview must wait for saved settings');
  const slider=()=>document.querySelector('#clip-chime-volume');
  const set=async value=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(slider(),String(value));slider().dispatchEvent(new Event('input',{bubbles:true}));await delay()};
  assert(slider().value==='100','Original volume default');
  await set(250);assert(document.querySelector('output[for="clip-chime-volume"]').textContent==='250%','Volume readout');
  await click(button('Test chime'));assert(preview===250,'Preview uses unsaved draft');assert(!saved,'Preview must not save');
  await set(0);assert(button('Test chime').disabled,'Mute disables preview');
  await set(250);
  document.querySelector('form').requestSubmit();await delay();
  assert(saved?.application.clipSoundVolume===250,'Volume persists with settings');
  assert(saved?.overlay.enabled,'HUD enable persists');
  assert(!button('Test notification').disabled,'Saved HUD can be tested');
  await click(document.querySelector('.settings-modal-close-btn'));
  await click(document.querySelector('[aria-label="Open Settings dialog"]'));await click(button('HUD & Sound'));
  assert(slider().value==='250','Saved volume survives reopen');
  await set(50);await click(document.querySelector('.settings-modal-close-btn'));
  await click(document.querySelector('[aria-label="Open Settings dialog"]'));await click(button('HUD & Sound'));
  assert(slider().value==='250','Discard restores saved volume');
  slider().scrollIntoView({block:'center'});await delay();
  return 'PASS: HUD enabled, chime draft preview, mute, save, reopen, discard';
})()
