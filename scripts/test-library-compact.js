// Evaluate in the collaborative Vite preview. Same-origin frames exercise real
// CSS viewports without depending on the preview host's docked viewport size.
(async () => {
  if (!location.search.includes('preview=1')) throw Error('Use the isolated preview.');
  const results=[];
  const assert=(value,message)=>{if(!value)throw Error(message);};
  for(const width of [960,1280,1440,1920,2560]) {
    const frame=document.createElement('iframe');
    frame.style.cssText=`position:fixed;left:0;top:0;width:${width}px;height:900px;border:0;z-index:1000000;background:#111;`;
    frame.src='/?preview=1';
    document.body.append(frame);
    try {
      const doc=await new Promise((resolve,reject)=>{frame.onload=()=>resolve(frame.contentDocument);setTimeout(()=>reject(Error('Preview frame did not load')),10000);});
      for(let i=0;i<100&&!doc.querySelector('.clip-card');i++)await new Promise(resolve=>setTimeout(resolve,40));
      const rect=selector=>doc.querySelector(selector).getBoundingClientRect();
      const heading=rect('.library-heading');
      const clips=rect('.clip-grid');
      const collection=rect('.library-collection-filter');
      const games=rect('.library-chips-bar');
      const toolbar=rect('.library-toolbar');
      const metadata=rect('.storage-summary');
      assert(frame.contentWindow.innerWidth===width,'Actual iframe viewport');
      assert(Math.abs(collection.y+collection.height/2-games.y-games.height/2)<2,`${width}: favorites and games share a row`);
      assert(games.left>=collection.right,`${width}: filter groups do not overlap`);
      assert(toolbar.right<=width && heading.left>=0,`${width}: controls remain within viewport`);
      assert(clips.top>=metadata.bottom,`${width}: clips do not overlap metadata`);
      const headerHeight=clips.top-heading.top;
      assert(headerHeight<(width>1800?100:width>1000?140:180),`${width}: compact header (${headerHeight})`);
      assert(doc.documentElement.scrollWidth<=width,`${width}: no horizontal document overflow`);
      results.push({width,headerHeight,filterY:collection.y,toolbarY:toolbar.y});
    } finally {frame.remove();}
  }
  return {passed:true,viewports:results};
})()
