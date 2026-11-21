document.head.insertAdjacentHTML('beforeend','<link rel="stylesheet" href="btn2.css">');
/* Sprawl button directions: shared markup. The same real set is drawn in every direction. */
const DIRS=[
 ['today','dir-1-today','1 · Today, tidied','The app as it is, made consistent.',
  'Keeps the solid blue primary (r6), the subtle grey secondary (r5) and the 12 px text. Tidied: one height for each level (32/28), one focus ring, a real disabled look, hover and pressed steps. "For good" stays the same blue as safe actions.',
  'Smallest change, no surprise for existing users. But it keeps the main weakness: deleting for good looks exactly like ticking the safe ones.'],
 ['fluent','dir-2-fluent','2 · Fluent accent levels','WinUI 3 accent, standard and subtle buttons, red for destructive.',
  'Windows system accent for the one primary action (#60CDFF dark, #0067C0 light), standard buttons with a hairline stroke, subtle (transparent) buttons for text actions, 4 px corners, 32 px high, 14 px text. Selected chips use the accent fill like a ToggleButton. "Delete for good" is a red button with a trash icon.',
  'Looks native and follows the user\'s Windows accent, so it feels at home next to light mode. A bit bigger than today: rows need more width (they wrap instead of clipping).'],
 ['outline','dir-3-outline','3 · Outlined','Only the primary is filled; everything else is an outline.',
  'Primary blue is the only filled button. Secondary, chips and scan are outlined (1 px), the danger button is a red outline that fills on hover, text buttons underline on hover. 6 px corners.',
  'Very clear who the main action is and calm to look at. Outlines add visual lines to dense screens (Advisor has many buttons), and disabled outlined buttons are easy to confuse with enabled ones.'],
 ['risk','dir-4-risk','4 · Risk-coloured primary','The main button takes the colour of what it does.',
  'Review selection is green when only safe items are ticked, amber when some need a check, and "Delete N for good" is red. Same colours as the Advisor risk bar, so the button repeats the risk you already see. Secondary buttons are plain grey; chips are neutral.',
  'The danger is visible before the click, and it teaches the colour language. Costs colour consistency (the primary changes colour) and needs the colours to be colour-blind safe, so every one keeps its text and icon.'],
 ['dense','dir-5-dense','5 · Dense toolbar','24 px controls, joined segmented chips, icon-led.',
  '24 px secondary and 28 px primary buttons, 3 px corners, 12 px text, icons next to labels, section/sort/show chips joined into one segmented control with a shared border.',
  'Fits the most on one line and suits power users; the clean-up bar stays on one row. Hit targets are small (24 px) and it feels less like Windows 11 touch-friendly Fluent.'],
 ['pill','dir-6-pill','6 · Tonal pills','Rounded tonal buttons: soft fills, pill chips with a check.',
  'Fully rounded buttons, a tonal (tinted) secondary instead of grey, a pale-blue primary in dark, pill chips that show a check when selected, 34 to 38 px high. Danger is a soft red.',
  'Friendly and modern, the selected chip is clear without relying on colour alone (the check). Furthest from WinUI 3 defaults, so more custom styling to maintain.'],
 ['aurora','dir-7-aurora','7 · Aurora gradient','Brand gradient primary with a soft glow.',
  'The logo gradient (blue to violet) fills the primary, a warm red-to-orange gradient the danger button, a teal-to-blue one Scan. Buttons get a soft coloured glow and an inner highlight, hover lifts the glow, pressed shrinks 3 %. Secondary and chips stay quiet glass; the selected chip gets a tinted gradient and a check. 10 px corners.',
  'Strongest brand feel, and the primary is unmistakable. Gradients and glow can look busy if every row has them, and light mode needs darker stops (done here). Custom brushes in XAML, not system styles.'],
 ['glass','dir-8-glass','8 · Acrylic glass','Translucent buttons that pick up the colour behind them.',
  'Buttons are frosted: blurred translucent fills, a bright 1 px inner edge, soft drop shadow. Primary tints blue, danger tints red, secondary is clear glass. The clean-up bar itself is acrylic. 12 px corners.',
  'Looks the most like Windows 11 acrylic and Mica and is pretty over the treemap or a coloured surface. Legibility depends on what is behind; WinUI acrylic needs real brushes and costs some rendering, and low contrast is the risk.'],
 ['raised','dir-9-raised','9 · Raised and tactile','Buttons with depth that visibly press in.',
  'Subtle top-lit gradients, a hard 2 px lower edge and soft shadow. On press the button drops 2 px and the edge disappears; selected chips look pushed in (inset), like a toggled hardware key. Primary blue, danger red, Scan teal.',
  'Very clear affordance: you see what is clickable, what is on, and the press feedback is satisfying. Dated against flat Fluent, and many raised buttons in a dense row get heavy.'],
 ['neon','dir-10-neon','10 · Neon glow','Dark-first outlines that glow cyan; red glow for danger.',
  'Dark tinted fills with a neon outline and glow (cyan primary, pink-red danger), secondary is a thin outline that lights up on hover, selected chips glow. In light mode it falls back to soft tinted fills with a focus-halo instead of glow.',
  'Distinctive, techy, and the danger glow is hard to miss. It is the loudest option, fits a "disk tool" mood but not Windows conventions, and glow does not translate to light mode well.'],
 ['ring','dir-11-ring','11 · Gradient ring','Dark fills with a gradient border; selected chips fill with the gradient.',
  'Primary and danger are dark (light: white) with a 1 px gradient border (blue to pink, red to amber); hover brightens the ring and adds a glow. Secondary has a neutral ring that turns to the gradient on hover. Selected chips are filled with the gradient and a check. Scan is a solid teal-blue gradient.',
  'Fancy without big filled gradients, so it reads calmer than Aurora and stays legible on any background. The ring is a layered brush (a bit more XAML work) and thin borders make disabled vs enabled less obvious.'],
 ['tile','dir-12-tile','12 · Icon tiles','Each button has a darker icon tile fused on its left.',
  'Primary, danger, secondary and Scan carry an icon in a tinted square tile at the left edge, so buttons read by icon first (trash, tick, back, scan). Chips get a coloured edge bar when selected. Otherwise flat Fluent colours, 6 px corners.',
  'Fast to scan, and the trash tile makes "for good" unmistakable even before reading. Wider buttons (rows wrap sooner) and every button needs a meaningful icon.']
];
const REC='fluent';

function B(kind,label,o={}){
 const ic=o.ic?`<i class="ic">&#x${o.ic};</i>`:'';
 return `<button type="button" class="b ${kind} ${o.cls||''}" ${o.risk?`data-risk="${o.risk}"`:''} ${o.dis?'disabled':''}>${ic}${label}</button>`;
}
function chips(items,on){return `<span class="grp">`+items.map((t,i)=>B('c',t,{cls:i===on?'on':''})).join('')+`</span>`}

function cleanbar(){return `<div class="bar">
 <div class="r"><span class="cnt num">17 ticked · up to 2.5 GiB</span>${B('p','Review selection...',{risk:'safe',ic:'E73E'})}<span class="vsep"></span>${B('s','Tick the safe ones',{ic:'E762'})}${B('s','Untick all',{ic:'E894'})}</div>
 <div class="dim num">On disk now 2.5 GiB · after at least 0 bytes · frees up to 2.5 GiB</div>
 <div class="r"><span class="dim">Or tick enough for</span><input class="tb num" value="20" aria-label="GB"><span class="dim">GB</span>${B('s','Tick')}</div>
 <div class="dim">The safe ones are ticked. Change the ticks, then review them: nothing goes before you confirm.</div>
 <div class="r">${B('s','Compare options for this goal')}<span class="ml">${B('t','Save as a script instead...')}</span></div></div>`}
function emptybar(){return `<div class="bar"><div class="r"><b style="font-size:13px;margin-right:6px">Nothing ticked</b>${B('s','Tick the safe ones',{ic:'E762'})}${B('s','Compare options for this goal')}${B('p','Review selection...',{dis:1,ic:'E73E'})}${B('s','Untick all',{dis:1})}</div></div>`}
function review(){return `<div class="bar"><div style="font-size:15px;font-weight:600">Review: 15 for good, 2 clean-ups · 3.2 GiB</div><div class="dim">Deleted for good means not in the Recycle Bin. You can still go back.</div>
 <div class="r">${B('d','Delete 15 for good, run 2 clean-ups',{risk:'bad',ic:'E74D'})}${B('s','Back',{ic:'E72B'})}</div></div>`}
function chiprows(){return `<div class="r" style="gap:22px"><div class="r"><span class="dim">Section</span>${chips(['Findings (27)','Growth','Remembered (0)'],0)}</div></div>
 <div class="r" style="margin-top:8px"><span class="dim" style="width:34px">Sort</span>${chips(['Recommended','Size','Risk','Owner'],0)}</div>
 <div class="r" style="margin-top:8px"><span class="dim" style="width:34px">Show</span>${chips(['All','Can clean now','My own files'],0)}<span class="ml">${B('s','Find duplicate files',{ic:'E8EF'})}</span></div>`}
function topbtns(){return `<div class="r"><span class="dim">Select:</span><span class="combo">[C:] Local Disk</span>${B('scan','Scan',{ic:'E721'})}<span class="vsep"></span>${B('s','Check again',{ic:'E72C'})}${B('s','Set up AI')}<span class="vsep"></span>${B('p','Update and restart',{ic:'E895'})}${B('s','Later')}${B('t','Skip this version')}</div>`}
function textbtns(){return `<div class="r">${B('t','See all in Cleaners')}${B('t','Save as a script instead...')}${B('t','(hide)')}${B('t','Details')}</div>`}
function rows(){return `<div class="rb"><span class="cb mix"></span><div><div class="nm">Apps built on Chromium (recognised)</div><div class="dim">247 parts · 10 ticked</div></div><span class="num">1.9 GiB</span></div>
 <div class="rb hov"><span class="cb on"></span><div><div class="nm">Developer tools</div><div class="dim">2 parts · 2 ticked · hovered</div></div><span class="num">1.2 GiB</span></div>
 <div class="rb foc"><span class="cb"></span><div><div class="nm">Graphics drivers</div><div class="dim">1 part · keyboard focus</div></div><span class="num">946.8 MiB</span></div>
 <div class="rb sel"><span class="cb on"></span><div><div class="nm">Windows</div><div class="dim">1 part · 1 ticked · selected</div></div><span class="num">281.6 MiB</span></div>`}
function states(){
 const R=[['Primary','p','Review'],['Secondary','s','Tick safe'],['Danger','d','Delete'],['Chip','c','Size'],['Chip selected','c on','Size'],['Text','t','See all']];
 const C=[['Normal',''],['Hover','hov'],['Pressed','prs'],['Focused','foc'],['Disabled','dis']];
 let h=`<div class="sg"><span></span>`+C.map(c=>`<span class="hd">${c[0]}</span>`).join('');
 R.forEach(r=>{h+=`<span class="rl">${r[0]}</span>`+C.map(c=>`<span class="cell">${B(r[1],r[2],{cls:c[1]==='dis'?'dis':c[1],dis:c[1]==='dis'})}</span>`).join('')});
 return h+`</div>`}

function sheet(dir,th,mini){
 const rk=dir==='risk'?' data-rk="1"':'';
 let h=`<div class="sheet" data-dir="${dir}" data-th="${th}"${rk}><h4>Clean-up bar, ${th}</h4>${cleanbar()}`;
 if(mini){return h+`<h4>Review</h4>${review()}<h4>Chips and scan</h4>${chiprows()}<div style="margin-top:10px">${topbtns()}</div></div>`}
 return h+`<h4>Nothing ticked yet (disabled buttons)</h4>${emptybar()}<h4>Review step</h4>${review()}<h4>Section, sort and show chips</h4>${chiprows()}<h4>Scan, Advisor and update buttons</h4>${topbtns()}<h4>Text buttons</h4>${textbtns()}<h4>Row buttons (a whole row is the button)</h4>${rows()}<h4>States</h4>${states()}</div>`;
}
function page(id){
 const i=DIRS.findIndex(d=>d[0]===id),d=DIRS[i];
 const nav=`<span class="nav"><a href="index.html">Compare all</a>${DIRS[i-1]?`<a href="${DIRS[i-1][1]}.html">‹ prev</a>`:''}${DIRS[i+1]?`<a href="${DIRS[i+1][1]}.html">next ›</a>`:''}</span>`;
 document.title='Sprawl buttons: '+d[2];
 document.body.innerHTML=`<div class="page"><div class="top"><h1>${d[2]}</h1>${id===REC?'<span class="tag rec">recommended</span>':''}${nav}</div>
 <p class="lead"><b>${d[3]}</b> ${d[4]}</p><p class="lead"><b>Trade-off:</b> ${d[5]}</p>
 <div class="ph">Dark (default)</div>${sheet(id,'dark')}<div class="ph">Light</div>${sheet(id,'light')}</div>`;
}
