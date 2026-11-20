/* Sprawl design mockups 2026-11-20: shared data + small component builders. Plain JS, no build step. */
const PAGES = [
 // group, file, title, summary, note(what changed / why)
 ['A','01-refined-dark','Refined dark','Same layout, tidier: 4 px grid, 8 px radii, quieter strokes, one accent.','<b>What changed:</b> cards get one 8 px radius and a hairline stroke, the three header cards line up on a 4 px grid, section captions are 12 px grey, the selected row uses a soft accent tint with a 3 px accent pill. Accent is the standard Windows blue (#60CDFF in dark).<br><b>Why it helps:</b> less visual noise, clearer hierarchy: the eye lands on the scan button, the selected path and the treemap first.'],
 ['A','02-refined-light','Refined light','The same refined window in Fluent light (Mica #F3F3F3, white cards).','<b>What changed:</b> light theme tokens: Mica #F3F3F3, cards white with a 6 % black stroke, accent #0067C0, risk colours darkened for contrast (green #0F7B0F, amber #9D5D00, orange #C4500A, red #C42B1C). Treemap tiles use lighter, less saturated fills with white gutters.<br><b>Why it helps:</b> light mode needs its own risk palette: the dark-mode colours fail contrast on white. Every colour here passes 4.5:1 for text.'],
 ['A','03-pivot-tabs','Underline tabs with counts','Pivot-style tab row: accent underline, counts on Advisor and Cleaners.','<b>What changed:</b> the five tab buttons become a Pivot: plain text, a 3 px accent underline on the active one, small count badges (Advisor 9, Cleaners 14). The filter box moves into the same row, right aligned.<br><b>Why it helps:</b> the active page is obvious without boxes, and the badges tell you there is something to do before you click.'],
 ['A','04-button-hierarchy','Button hierarchy','Scan is the one accent button; the map bar becomes a segmented control + icon buttons.','<b>What changed:</b> only Scan is filled with the accent colour. Treemap / City / Galaxy become one segmented control. Colour, Tweaks and History turn into icon buttons with tooltips and a divider between groups. Zoom and More views move into the same toolbar.<br><b>Why it helps:</b> one primary action per screen, and the mode switch reads as a choice of one rather than seven equal buttons.'],
 ['A','05-brand-accent','Brand accent (sunset)','Accent taken from the logo gradient: orange to violet instead of default blue.','<b>What changed:</b> selection, links, the usage bar and the Scan button use the logo\'s orange-to-violet gradient (#FF8A4C to #8E54E9). Neutral surfaces stay Fluent greys.<br><b>Why it helps:</b> gives Sprawl its own identity while staying inside Fluent layout and contrast rules. A single swap of the accent token; WinUI lets you set <i>SystemAccentColor</i> in the app resources.'],
 ['A','06-comfortable-rows','Comfortable rows + selection bar','36 px rows with inline usage bars; the selection bar becomes a floating action bar.','<b>What changed:</b> table rows go from about 26 to 36 px with a thin usage bar under the name, sizes in semibold, and a 12 px caption of path and item count. Selecting rows raises a floating bar: "3 selected, 12.4 GB: Show on map, Clean, Delete".<br><b>Why it helps:</b> larger hit targets, easier scanning, and the destructive actions appear only when needed, next to what you selected.'],
 ['B','07-commandbar','CommandBar instead of the menu','Menu bar replaced by a CommandBar: Scan, Rescan, Export, Options, Help.','<b>What changed:</b> File / Options / Help disappear. A CommandBar carries the common commands with labels (Scan, Rescan, Open in Explorer, Save image, Settings) and a "..." overflow with the rest. Keyboard shortcuts show as accelerator text.<br><b>Why it helps:</b> frequent actions are one click instead of two; the menu bar is a desktop-1995 pattern that WinUI 3 itself moves away from.'],
 ['B','08-scan-split-button','Scan split button + breadcrumb','Drive picker and Scan merge into one split button; the path becomes a breadcrumb with usage.','<b>What changed:</b> the 3-card header collapses into one slim row: a Scan split button (main click scans the last target, the arrow lists drives, Folder... and recent scans), a breadcrumb bar (C: > Users > sam) and a compact usage meter. Logo moves to the title bar.<br><b>Why it helps:</b> saves 70 px of height for the map and puts "where am I" and "how full is it" on one line.'],
 ['B','09-titlebar-search','Search in the title bar','Filter box lives in the title bar as an AutoSuggestBox; tabs go under the header.','<b>What changed:</b> the Filter box moves into the custom title bar (the draggable area stays free on both sides) and becomes an AutoSuggestBox with suggestions: recent filters, file types, "Search everywhere...". Ctrl+E focuses it.<br><b>Why it helps:</b> the filter is global to the app, so it belongs at app level, not in a tab row; frees the tab row.'],
 ['B','10-infobars','InfoBars and a TeachingTip','Notices as InfoBars under the header; first-run hint as a TeachingTip.','<b>What changed:</b> update available, partial scan (some folders need elevation), privacy-cleaners hint and AI-setup nag become stacked InfoBars with severity colours and one action each. A TeachingTip points at Advisor the first time a scan finishes.<br><b>Why it helps:</b> one consistent place and style for "something needs your attention", dismissible, never covering the data.'],
 ['B','11-details-panel','Details panel on the right','Selection opens an inspector: type breakdown, owner, age, actions.','<b>What changed:</b> a 320 px inspector docks right (toggle with Alt+Enter). It shows the selected folder\'s size, share of drive, a mini treemap, its top file types, last modified, owner and the actions: Show in Explorer, Show on map, Ask Advisor, Delete.<br><b>Why it helps:</b> moves the context menu\'s content into a visible place, so you see what a folder is before you act on it.'],
 ['B','12-map-overlay','Map toolbar overlay + status bar','Map bar floats over the treemap as an acrylic pill; footer becomes a real status bar.','<b>What changed:</b> the map bar leaves its own row and floats top-centre over the treemap as an acrylic pill (it fades when idle). The footer becomes a status bar: items scanned, scan time, hovered path, selection size, and a legend chip.<br><b>Why it helps:</b> the map gets the 44 px the bar used, and the controls sit where they act.'],
 ['C','13-nav-rail','NavigationView: compact rail','Tabs replaced by a left NavigationView rail (icons) with Settings at the bottom.','<b>What changed:</b> Tree, Files, Advisor, Cleaners and Search move to a 48 px rail (Mica-backed, selected item gets the accent indicator). Settings and Help at the bottom. The tab row disappears; the header becomes a title row.<br><b>Why it helps:</b> frees a full row and scales when more pages appear (History, Compare, Cleaners subpages).'],
 ['C','14-nav-drives','NavigationView: drives as sections','Expanded NavigationView with drives and recent scans, then tools.','<b>What changed:</b> an expanded 260 px pane: Home, Drives (C:, D:, E: with free-space meters), Recent scans, then Tools (Advisor, Cleaners, Search). Choosing a drive scans or opens its last result.<br><b>Why it helps:</b> the drive picker stops being a combo box: all drives are visible with their fullness, so you pick the fullest one at a glance.'],
 ['C','15-map-first','Map-first layout','Treemap takes about 60 % of the window; tree is a narrow side pane, file types are chips.','<b>What changed:</b> the treemap is the hero (large, with breadcrumb + toolbar on top). The folder tree is a 340 px pane on the left; file types become a horizontal chip strip with sizes under the map, clicking a chip lights up those files.<br><b>Why it helps:</b> Sprawl\'s differentiator is the map, so it gets the room; file types are a filter, not a table.'],
 ['C','16-tree-with-types','Tree with inline type strips','One table: each folder row carries a stacked bar of what it is made of.','<b>What changed:</b> the separate file-types pane is gone; each folder row shows a stacked colour bar (video / archives / code / system / other). A Types toggle at the top swaps the table to a file-type view with a folder breakdown on the right.<br><b>Why it helps:</b> answers "what is this folder full of?" without looking at a second pane.'],
 ['C','17-home-dashboard','Home dashboard','Start page with drive cards, what grew, recent scans and first suggestions.','<b>What changed:</b> instead of an empty tree: drive cards with big free-space numbers and Scan buttons, a "grew since last scan" list, recent scans with ages, and the top 3 Advisor suggestions. Accent button "Scan C:".<br><b>Why it helps:</b> the first screen answers "what should I do now?" and gives every drive a one-click path.'],
 ['C','18-advisor-3col','Advisor: findings / details / AI','Three columns: findings list, selected finding details, Ask-your-disk chat.','<b>What changed:</b> findings grouped by risk with checkboxes on the left; the middle column explains the selected finding (why it is safe, what it contains, command preview, Show in tree); the right column is the AI chat, collapsible. A sticky footer shows "Tick all safe: 16.2 GB" and Review and approve.<br><b>Why it helps:</b> no popups: browse, read, decide, act in one place.'],
 ['C','19-advisor-cards','Advisor: summary and cards','A big "can free 46 GB" header with risk filter chips and expandable finding cards.','<b>What changed:</b> the page opens with a summary ("You can free 46.2 GB, 16.2 GB of it is safe"), a stacked risk bar acting as filter, and finding Expanders (title, size, risk pill; body has details and actions).<br><b>Why it helps:</b> motivation first, detail on demand: good for people who do not read tables.'],
 ['C','20-cleaners-grid','Cleaners as cards','Cleaner list becomes a card grid with ToggleSwitches and an Expander for details.','<b>What changed:</b> each cleaner (Windows Update cache, Temp, Thumbnails, Browser caches, Recycle Bin, Delivery Optimisation) is a card with size, ToggleSwitch, risk dot and "What is this?" Expander. A bottom CommandBar: Select safe, Clean 7.8 GB.<br><b>Why it helps:</b> toggles are quicker than checkboxes in a list, and the total updates in a persistent action bar.'],
 ['C','21-search-builder','Search with a query builder','Search page with filter chips (size, age, type, folder) above the results.','<b>What changed:</b> the search box becomes a query line with removable chips (Larger than 500 MB, Older than 1 year, Type: video, In: D:\\), a result table with a preview strip, and a "Save as filter" button.<br><b>Why it helps:</b> powerful search without learning syntax; the syntax is still shown for those who want it.'],
 ['C','22-scanning','Scanning state','Live progress: ring, counters, partial treemap growing, cancel.','<b>What changed:</b> the scan page shows a determinate ProgressBar, files per second, the folder being read, a live partial treemap and the biggest folders found so far. Stop is a standard button; an InfoBar notes that elevated access would read more.<br><b>Why it helps:</b> a long scan feels alive and you can already explore before it finishes.'],
];

const K = {
 ic:(c,cls='')=>`<i class="ic ${cls}">&#x${c};</i>`,
 logo(big){const b=[[18,30],[32,52],[46,40],[60,64],[74,28]].map(([l,h])=>`<i style="left:${l}%;height:${h}%"></i>`).join('');return `<span class="logo ${big?'big':''}">${b}</span>`},
 cap:`<div class="cap-btns"><span>&#xE921;</span><span>&#xE922;</span><span>&#xE8BB;</span></div>`,
 tb(mid='',left=''){return `<div class="tb">${K.logo()}<span>Sprawl</span>${left}<div class="sp"></div>${mid}<div class="sp"></div>${K.cap}</div>`},
 menu:`<div class="menu"><span>File</span><span>Options</span><span>Help</span></div>`,
 tabs(variant='',items,on=0){
  items=items||[['Tree View'],['File View'],['Advisor'],['Cleaners'],['Search','E721']];
  return `<div class="tabs ${variant}">`+items.map((t,i)=>`<span class="tab ${i===on?'on':''}">${t[1]?K.ic(t[1],'s'):''}${t[0]}${t[2]?`<span class="count">${t[2]}</span>`:''}</span>`).join('')+`</div>`},
 btn:(label,ic,cls='')=>`<span class="btn ${cls}">${ic?K.ic(ic):''}${label||''}</span>`,
 ibtn:(ic,cls='')=>`<span class="btn ico sub ${cls}">${K.ic(ic)}</span>`,
 TREE:[
  [0,'C:\\',389.4,100,'1.42 M',1],[1,'Users',148.2,38,'612 k',1],[2,'sam',146.9,99,'608 k',1],[3,'AppData',71.3,49,'402 k',1],
  [4,'Local',58.4,82,'344 k',1],[5,'Docker',21.6,37,'1.2 k'],[5,'Temp',14.2,24,'81 k'],[5,'npm-cache',9.8,17,'52 k'],[4,'Roaming',12.9,18,'58 k'],
  [3,'Documents',38.1,26,'96 k'],[3,'Downloads',22.4,15,'1.1 k'],[3,'Videos',11.0,7,'214'],[3,'.cargo',4.8,3,'41 k'],
  [1,'Public',1.3,1,'94'],[0,'Windows',41.7,11,'287 k'],[0,'Program Files',33.5,9,'88 k'],[0,'Program Files (x86)',18.9,5,'61 k'],[0,'ProgramData',12.6,3,'42 k'],
  [0,'hiberfil.sys',12.8,3,'1',0,1],[0,'pagefile.sys',8.0,2,'1',0,1],[0,'$Recycle.Bin',3.1,1,'218']],
 tree({sel=3,rowh=30,rows,cols}={}){
  rows=rows||K.TREE;
  const h=`<div class="th"><div>Name</div><div>Usage</div><div class="rt">Size</div><div class="rt">%</div><div class="rt">Files</div></div>`;
  const b=rows.map((r,i)=>`<div class="tr ${i===sel?'sel':''} ${r[6]?'file':''}" style="--rowh:${rowh}px"><div class="nm" style="padding-left:${r[0]*16}px">${r[6]?'<i class="ic ch"></i>':`<i class="ic ch">${r[5]?'&#xE70D;':'&#xE76C;'}</i>`}<i class="ic fi">${r[6]?'&#xE7C3;':'&#xE8B7;'}</i><span class="ell">${r[1]}</span></div><div class="pcbar"><i style="width:${r[3]}%"></i></div><div class="rt num">${r[2].toFixed(1)} GB</div><div class="rt num dim2">${r[3]} %</div><div class="rt num dim">${r[4]}</div></div>`).join('');
  return `<div class="tbl t-tree fill">${h}<div class="scroll fill">${b}</div></div>`},
 TYPES:[['.mp4',41.2,'312','#e8708a'],['.dll',22.8,'61 k','#6aa9ff'],['.vhdx',21.6,'3','#b58cff'],['.zip',14.3,'1.2 k','#f6b042'],['.exe',13.9,'9.8 k','#5fd3b3'],['.iso',9.1,'4','#ff8f5a'],['.rlib',8.4,'7.3 k','#d7d35a'],['.png',7.7,'48 k','#7fd0ff'],['.pak',6.5,'86','#c58a6a'],['.log',4.2,'3.1 k','#9aa6b8'],['.js',3.9,'212 k','#e5c95a'],['(no ext)',3.1,'66 k','#8893a6']],
 types({sel=0,rowh=30}={}){
  const mx=41.2;
  const b=K.TYPES.map((t,i)=>`<div class="tr ${i===sel?'sel':''}" style="--rowh:${rowh}px"><span class="sw" style="background:${t[3]}"></span><div class="semi">${t[0]}</div><div class="pcbar"><i style="width:${t[1]/mx*100}%;background:${t[3]}"></i></div><div class="rt num">${t[1].toFixed(1)} GB</div><div class="rt num dim">${t[2]}</div></div>`).join('');
  return `<div class="tbl t-types fill"><div class="th"><div></div><div>Type</div><div>Usage</div><div class="rt">Size</div><div class="rt">Files</div></div><div class="scroll fill">${b}</div></div>`},
 TM:[
  [210,'Users',[['Projects',24.8],['Downloads',22.4],['Docker',21.6],['Temp',14.2],['Videos',11.0],['npm-cache',9.8],['Pictures',6.3],['.cargo',4.8],['Other',33]]],
  [215,'Windows',[['System32',14.9],['WinSxS',11.2],['Installer',8.1],['SoftwareDist.',4.6],['Other',2.9]]],
  [140,'Program Files',[['Steam',18.4],['Adobe',7.3],['JetBrains',3.5],['Other',4.3]]],
  [280,'Program Files (x86)',[['Games',9.5],['Other',9.4]]],
  [20,'hiberfil.sys',[['hiberfil.sys',12.8]]],[330,'ProgramData',[['Docker',6.1],['Package Cache',3.9],['Other',2.6]]],
  [60,'pagefile.sys',[['pagefile.sys',8.0]]]],
 squarify(items,x,y,w,h){
  const total=items.reduce((a,b)=>a+b.v,0),sc=w*h/total,out=[];let rest=items.map(i=>({...i,a:i.v*sc}));
  const worst=(row,s)=>{const t=row.reduce((a,b)=>a+b.a,0),mx=Math.max(...row.map(r=>r.a)),mn=Math.min(...row.map(r=>r.a));return Math.max(s*s*mx/(t*t),t*t/(s*s*mn))};
  while(rest.length){const s=Math.min(w,h);let row=[rest[0]],i=1;
   while(i<rest.length&&worst([...row,rest[i]],s)<=worst(row,s)){row.push(rest[i]);i++}
   rest=rest.slice(row.length);const t=row.reduce((a,b)=>a+b.a,0);
   if(w>=h){const cw=t/h;let cy=y;row.forEach(r=>{const ch=r.a/cw;out.push({...r,x,y:cy,w:cw,h:ch});cy+=ch});x+=cw;w-=cw}
   else{const ch=t/w;let cx=x;row.forEach(r=>{const cw=r.a/ch;out.push({...r,x:cx,y,w:cw,h:ch});cx+=cw});y+=ch;h-=ch}}
  return out},
 tm({aspect=1.7,hot='',flat=false,labels=true,only=null}={}){
  const W=100*aspect,H=100;
  const data=K.TM.filter(g=>!only||only.includes(g[1]));
  const tops=K.squarify(data.map(g=>({hue:g[0],name:g[1],kids:g[2],v:g[2].reduce((a,b)=>a+b[1],0)})).sort((a,b)=>b.v-a.v),0,0,W,H);
  let html='';
  tops.forEach(t=>{const kids=K.squarify(t.kids.map(k=>({name:k[0],v:k[1]})).sort((a,b)=>b.v-a.v),t.x,t.y,t.w,t.h);
   kids.forEach((k,i)=>{const px=v=>(v/W*100).toFixed(2),py=v=>(v/H*100).toFixed(2);
    const big=k.w/W>0.07&&k.h/H>0.09;
    html+=`<div class="t ${hot&&k.name===hot?'hot':''}" style="left:${px(k.x)}%;top:${py(k.y)}%;width:${px(k.w)}%;height:${py(k.h)}%;--h:${t.hue};--o:${(i%4)*2-2}">${labels&&big?`<b>${k.name}</b><span class="num">${k.v.toFixed(1)} GB</span>`:''}</div>`})});
  return `<div class="tm ${flat?'flat':''}" style="width:100%;height:100%">${html}</div>`},
 FIND:[
  ['g','npm cache','Safe to delete',9.8,1],['g','Windows Temp files','Safe to delete',4.1,1],['g','Thumbnail and shader caches','Safe to delete',2.3,1],
  ['a','Docker Desktop disk image','Takes time to get back',21.6,0],['a','Old installers in Downloads','Takes time to get back',6.3,0],
  ['o','Cargo target folders (4 projects)','Build outputs of projects you are working on',17.2,0],['o','Old Visual Studio versions','Rule files not used',3.2,0],
  ['r','Duplicate videos in D:\\Media','My own files',8.9,0],['r','Hibernation file','Cannot be undone',12.8,0]],
 find({sel=0,rowh=52,checks=true,from=0,to=99}={}){
  const lab={g:'Safe',a:'Check first',o:'Look first',r:'Careful'};
  return K.FIND.map((f,i)=>i<from||i>=to?"":`<div class="tr ${i===sel?'sel':''}" style="--rowh:${rowh}px;grid-template-columns:${checks?'22px ':''}12px minmax(0,1fr) 80px">${checks?`<span class="cbx ${f[4]?'on':''}">${f[4]?'&#xE73E;':''}</span>`:''}<span class="dot ${f[0]}"></span><div style="min-width:0"><div class="ell semi">${f[1]}</div><div class="cap dim ell">${f[2]}</div></div><div class="rt num semi">${f[3].toFixed(1)} GB</div></div>`).join('')},
 status(extra=''){return `<div class="status"><span>${K.ic('E8B7')} 1,418,206 items</span><span>${K.ic('E916')} Scanned in 8.4 s</span><span class="ell dim">C:\\Users\\sam\\AppData\\Local\\Docker</span><div class="sp"></div>${extra}<span>389.4 of 476 GB used</span></div>`},
 usage(p=82,cls=''){return `<div class="use ${cls}"><i style="width:${p}%"></i></div>`},
 /* gallery chrome */
 init(file){
  const q=new URLSearchParams(location.search),th=q.get('theme');
  if(th)document.documentElement.dataset.theme=th;
  if(q.has('thumb'))document.body.classList.add('thumb');
  const i=PAGES.findIndex(p=>p[1]===file),p=PAGES[i],g={A:'Slight',B:'Small redesign',C:'Bigger redesign'}[p[0]];
  const nav=(j)=>PAGES[j]?`<a href="${PAGES[j][1]}.html">${j<i?'‹ '+PAGES[j][1].slice(0,2):PAGES[j][1].slice(0,2)+' ›'}</a>`:'';
  const m=document.createElement('div');m.className='meta';
  m.innerHTML=`<a href="index.html">Gallery</a><b>${p[1].slice(0,2)} · ${p[2]}</b><span class="badge">${g}</span><span class="sp"></span>${nav(i-1)}${nav(i+1)}<button id="thm">Light / dark</button>`;
  document.body.prepend(m);
  const n=document.createElement('section');n.className='note';
  n.innerHTML=`<h2>${p[2]}</h2><p>${p[4]}</p>`;document.body.append(n);
  const apply=()=>{const t=document.documentElement.dataset.theme;q.set('theme',t);history.replaceState(null,'','?'+q);document.querySelectorAll('.meta a').forEach(a=>{const h=a.getAttribute('href').split('?')[0];a.href=h+'?theme='+t})};
  document.getElementById('thm').onclick=()=>{const d=document.documentElement;d.dataset.theme=d.dataset.theme==='dark'?'light':'dark';apply()};
  if(th)apply();
 }
};

/* ---- composite parts, used by many pages ---- */
K.filter=(w=340,focus=false)=>`<div class="input ${focus?'focus':''}" style="width:${w}px">${K.ic('E721','s')}<span class="ell">Filter: *.mp4;  node_modules;  *\\cache\\*</span></div>`;
K.hdr=(o={})=>`<div class="row g12" style="padding:4px 12px 8px;align-items:stretch">
 <div class="card pad col g8" style="width:380px">
  <div class="row g8"><span class="dim2">Select:</span><div class="combo fill">${K.ic('EDA2')}<span>C:\\ (Windows)  476 GB</span>${K.ic('E70D')}</div>${K.btn('Scan','E721','acc')}</div>
  <div class="cap dim2">Scanned C:\\ in 8.4 s. 1,418,206 items.</div><div class="use thin acc"><i style="width:100%"></i></div></div>
 <div class="card pad fill col g8" style="justify-content:center">
  <div class="row g12"><span class="lbl">Selected:</span><span class="semi cap ell">C:\\Users\\sam\\AppData</span></div>
  <div class="row g12 cap dim2"><span>71.3 GB, 402,118 files</span><span class="acc-t" style="color:var(--acc)">Details</span></div>
  <div class="row g12"><span class="lbl">Drive:</span><span class="cap dim2">C: 389.4 of 476 GB used, 86.6 GB free</span><div class="fill">${K.usage(82)}</div></div></div>
 <div class="card pad row g12" style="padding-right:22px">${K.logo(true)}<div><div class="h3">Sprawl <span class="dim" style="font-weight:400;font-size:14px">1.0.5</span></div><div class="cap dim">Disk space analyser</div></div></div></div>`;
K.mapbar=(v='old')=>{
 if(v==='old')return `<div class="row g8" style="padding:0 12px 6px">${['Treemap|F0E2|on','City|E80F','Galaxy|E7FC'].map(s=>{s=s.split('|');return K.btn(s[0],s[1],'sm '+(s[2]||''))}).join('')}<span class="divider"></span>${K.btn('More views','E712','sm')}${K.btn('Colour','E790','sm')}${K.btn('Tweaks','E713','sm')}${K.btn('History','E81C','sm')}</div>`;
 return `<div class="row g8" style="padding:0 12px 8px"><div class="seg">${K.btn('Treemap','F0E2','on')}${K.btn('City','E80F')}${K.btn('Galaxy','E7FC')}</div>${K.btn('More views','E712','sm sub')}<span class="divider"></span>${K.ibtn('E8A3')}${K.ibtn('E71F')}${K.ibtn('E790')}${K.ibtn('E713')}<span class="sp"></span>${K.ibtn('E81C')}${K.btn('Save image','E74E','sm sub')}</div>`};
K.classic=(o={})=>{
 const rowh=o.rowh||30;
 return `${o.tbar||K.tb()}${o.top!==undefined?o.top:(o.menu===false?'':K.menu)}${o.hdr||K.hdr()}${o.infos||''}
 <div class="row g12" style="padding:0 12px 6px;min-height:36px">${K.tabs(o.tabs||'fill',o.tabItems,0)}<span class="sp"></span>${o.nofilter?'':K.filter(340)}</div>
 <div class="row g12" style="padding:0 12px 8px;height:${o.panesH||236}px"><div class="card col" style="flex:1.4;min-width:0">${K.tree({rowh})}</div><div class="card col" style="flex:1;min-width:0">${K.types({rowh})}</div></div>
 ${o.map==='none'?'':K.mapbar(o.map||'old')}
 <div class="fill" style="padding:0 12px 8px;position:relative">${K.tm({aspect:o.aspect||4.2,hot:'Docker'})}${o.over||''}</div>
 ${o.status||K.status()}`};

K.NAV=[['Home','E80F'],['Tree View','E8B7'],['File View','E7C3'],['Advisor','EA80','9'],['Cleaners','EA99'],['Search','E721']];
K.rail=(on)=>`<div class="nav col" style="width:56px;flex:none;align-items:center;padding-top:8px"><span class="ni rail">${K.ic('E700')}</span>`+K.NAV.map((n,i)=>`<span class="ni rail ${i===on?'on':''}" title="${n[0]}">${K.ic(n[1])}</span>`).join('')+`<div class="sp"></div><span class="ni rail">${K.ic('E897')}</span><span class="ni rail">${K.ic('E713')}</span></div>`;
K.navfull=(on)=>`<div class="nav col" style="width:264px;flex:none;padding:8px 8px 8px"><span class="ni">${K.ic('E700')}</span>`+
 K.NAV.slice(0,1).map((n,i)=>`<span class="ni ${on===0?'on':''}">${K.ic(n[1])}${n[0]}</span>`).join('')+
 `<div class="nhead">Drives</div>`+[['C:','Windows',82,1],['D:','Data',41,0],['E:','Backup',73,0]].map(d=>`<div class="ni ${on===1&&d[3]?'on':''}" style="height:52px">${K.ic('EDA2')}<div class="fill"><div>${d[0]} <span class="dim" style="font-weight:400">${d[1]}</span></div><div class="use thin" style="margin-top:6px"><i style="width:${d[2]}%"></i></div></div></div>`).join('')+
 `<div class="nhead">Tools</div>`+K.NAV.slice(1).map((n,i)=>`<span class="ni ${on===i+2?'on':''}">${K.ic(n[1])}${n[0]}${n[2]?`<span class="sp"></span><span class="count">${n[2]}</span>`:''}</span>`).join('')+
 `<div class="sp"></div><span class="ni">${K.ic('E713')}Settings</span></div>`;
