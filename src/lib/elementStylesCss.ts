// 阅读元素样式：按 .reader[data-el-<key>="<value>"] 命中的静态特效表，随 buildDynamicCss 注入。
// 选择器约定：
// - 一律带 .reader[data-el-*] 前缀。原书样式表（未开「覆盖书籍排版」时）会被后端加 #chapter-content
//   前缀后内联进正文，只有比它更高特异性的规则才能稳定盖住；同理也盖住阅读器组件里的基础样式。
// - 与 buildDynamicCss 的基础 !important 规则冲突的属性（标题的 text-align/margin/font-size、
//   段落的 text-indent/margin/font-size/line-height 等）必须同用 !important 且特异性更高。
// - 其余属性尽量不用 !important，保证设置页的「自定义 CSS」仍能按预期覆盖。
// 应用 DOM 与网页原型的差异：
// - 标题：第一标题标记 .tp-el-heading（可能是 h1~h6 或 p/div），其余标题 .tp-el-subhead；
//   原型里 h2 内的 .kicker 副行在 app 中不存在。
// - 首段：标记 .tp-el-lead（p 或 div）。
// - 插图：真实内容是 figure / 图片容器（.tp-el-figure），图注是 figcaption/p（.tp-el-caption）。
// - 链接：书内 <a>；强调：em/strong/b/i。
// - 页眉页脚：.topbar（.btitle/.stat-item/.stat-sep）+ .footer（.finfo/.fitem/.fsep）。
export const ELEMENT_STYLE_CSS = `/* 阅读元素特效样式：由设计参考稿转换为应用选择器（.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-*]），随动态 CSS 注入。 */
/* ==========================================================================
   0. 公共标记与基础（不依赖选项，始终生效）
   ========================================================================== */
/* 分页避让：图与图注不跨页拆分。 */
#chapter-content :is(.tp-el-figure, figure, img) { break-inside: avoid; }
#chapter-content :is(.tp-el-figure, figure) { break-after: avoid; }
/* 图注：真实 DOM 是 figcaption / p.tp-el-caption，会正文段落基础规则
   （#chapter-content p 的 font-size/text-indent/margin !important）压掉，这里用更高特异性还原。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image] #chapter-content :is(p, div, figcaption).tp-el-caption,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image] #chapter-content .tp-el-caption {
  display: block !important;
  margin: .4em 0 1.1em !important;
  text-align: center !important;
  text-indent: 0 !important;
  color: color-mix(in srgb, var(--rdr-fg) 62%, transparent) !important;
  background: transparent !important;
  font-family: var(--sans) !important;
  font-size: .84em !important;
  line-height: 1.6 !important;
  break-inside: avoid;
}
/* 正文里的脚注/尾注上标链接：点击后由阅读器浮层展示注释。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote] #chapter-content .tp-el-noteref {
  color: var(--fx-accent, #8a5a3b) !important;
  cursor: pointer;
  text-decoration: none !important;
  font-size: .72em !important;
  vertical-align: super;
  line-height: 0 !important;
  padding: 0 .08em;
  border-radius: 2px;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote] #chapter-content .tp-el-noteref:hover {
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 12%, transparent) !important;
}
/* 脚注浮层基础：位置/尺寸由组件内联样式给出，这里只负责观感。 */
.tp-fn-pop {
  position: fixed;
  z-index: 600;
  width: min(380px, calc(100vw - 24px));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--rdr-line, #d8cbbd);
  border-radius: 10px;
  background: var(--rdr-surface, #fbf4ea) !important;
  color: var(--rdr-fg, #2b2520) !important;
  box-shadow: 0 18px 56px #1c17122e;
  font-family: var(--serif) !important;
  font-size: 13.5px !important;
  line-height: 1.75 !important;
}
.tp-fn-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex: 0 0 auto;
  gap: 10px;
  padding: 8px 10px 7px 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--rdr-fg, #2b2520) 11%, transparent);
  color: color-mix(in srgb, var(--rdr-fg, #2b2520) 62%, transparent) !important;
  font-family: var(--sans) !important;
  font-size: 10px !important;
  letter-spacing: .16em;
}
.tp-fn-head .tp-fn-close { width: 24px; height: 24px; flex: 0 0 auto; }
.tp-fn-body {
  min-height: 0;
  overflow: auto;
  padding: 11px 14px 14px;
}
.tp-fn-body p { margin: .35em 0 !important; text-indent: 0 !important; font-size: inherit !important; line-height: inherit !important; }
.tp-fn-body :is(h1, h2, h3, h4, h5, h6) { margin: .5em 0 .25em; font-size: 1.05em; line-height: 1.4; }
.tp-fn-body ul, .tp-fn-body ol { margin: .35em 0; padding-left: 1.35em; }
.tp-fn-body :is(em, strong, b, i) { color: inherit; }
.tp-fn-body a { color: var(--fx-accent, #8a5a3b); }
.tp-fn-loading, .tp-fn-empty { padding: 16px; color: color-mix(in srgb, var(--rdr-fg, #2b2520) 55%, transparent); text-align: center; }
/* 排版硬修复（src/lib/readFix.ts 的 __read_fix）会给 #chapter-content 内所有元素写入
   font-family / font-size / line-height / color / background 的 !important（且背景是逐元素不透明纸色）。
   元素样式在同一批属性上必须同用 !important 且特异性更高才能生效；带底色的容器/图文卡片
   还要把内部元素的底色强制透明，否则底色会被逐个孩子元素的纸色盖住。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading]:not([data-el-heading="default"]) #chapter-content :is(.tp-el-heading, h2) :is(span, small, em, strong, b, i) { color: inherit !important; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead]:not([data-el-subhead="default"]) #chapter-content :is(.tp-el-subhead, h3) :is(span, small, em, strong, b, i) { color: inherit !important; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link]:not([data-el-link="default"]) #chapter-content a:not(.tp-el-heading-link) :is(span, em, strong, b, i) { color: inherit !important; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em]:not([data-el-em="default"]) #chapter-content :is(em, strong, b, i, mark.fx) :is(span, em, strong, b, i) { color: inherit !important; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image] #chapter-content .tp-el-caption :is(span, em, strong, b, i) { color: inherit !important; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table]:not([data-el-table="default"]) #chapter-content :is(th, td) :is(span, em, strong, b, i) { color: inherit !important; background: transparent !important; }

/* ==========================================================================
   1. 章节标题（data-el-heading）
   ========================================================================== */
/* 高特异性前缀同时用于压过原书样式；position 供编号/胶带等伪元素定位。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading] #chapter-content :is(.tp-el-heading, h2) {
  position: relative;
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="rule"] #chapter-content :is(.tp-el-heading, h2) {
  padding-bottom: .45em;
  border-bottom: 1px solid color-mix(in srgb, var(--rdr-fg) 24%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="double"] #chapter-content :is(.tp-el-heading, h2) {
  padding-bottom: .55em;
  border-bottom: 3px double color-mix(in srgb, var(--rdr-fg) 26%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="plain"] #chapter-content :is(.tp-el-heading, h2) {
  border-bottom: 0;
  padding-bottom: 0;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="numbered"] #chapter-content :is(.tp-el-heading, h2) {
  text-align: left !important;
  padding: 0 0 0 2.75em;
  border-bottom: 0;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="numbered"] #chapter-content :is(.tp-el-heading, h2)::before {
  content: var(--el-chapter, "03");
  position: absolute;
  left: 0;
  top: -.08em;
  font-family: var(--serif) !important;
  font-size: 2.1em !important;
  line-height: 1 !important;
  font-weight: 700;
  color: color-mix(in srgb, var(--fx-accent, #8a5a3b) 30%, transparent) !important;
  letter-spacing: -.04em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="gradient"] #chapter-content :is(.tp-el-heading, h2) {
  border-bottom: 0;
  padding-bottom: .65em;
  background: linear-gradient(90deg, var(--fx-accent, #8a5a3b), color-mix(in srgb, var(--fx-accent, #8a5a3b) 55%, #d4a373)) !important;
  -webkit-background-clip: text !important;
  background-clip: text !important;
  -webkit-text-fill-color: transparent !important;
  color: transparent !important;
}
/* MOBI 标题常被 <font><b><span> 包裹；readFix 会给所有后代铺纸色背景，
   把渐变裁字盖成空白。渐变标题的后代必须透明，让 h2 的 background-clip:text 透出来。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="gradient"] #chapter-content :is(.tp-el-heading, h2) :is(font, b, strong, em, span, a) {
  background: transparent !important;
  background-color: transparent !important;
  background-image: none !important;
  -webkit-text-fill-color: transparent !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="gradient"] #chapter-content :is(.tp-el-heading, h2)::after {
  content: "";
  display: block;
  width: 3.2em;
  height: 2px;
  margin: .5em auto 0;
  background: linear-gradient(90deg, transparent, var(--fx-accent, #8a5a3b), transparent) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="seal"] #chapter-content :is(.tp-el-heading, h2) {
  display: table;
  margin: .9em auto 1.2em !important;
  padding: .5em .8em;
  border: 2px solid var(--fx-accent, #8a5a3b) !important;
  box-shadow: inset 0 0 0 3px var(--rdr-bg), inset 0 0 0 4px color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="tape"] #chapter-content :is(.tp-el-heading, h2) {
  border-bottom: 0;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 11%, color-mix(in srgb, var(--rdr-bg) 88%, #ffffff)) !important;
  padding: .55em .85em .5em;
  border-radius: 4px;
  box-shadow: 0 3px 10px #0000000a;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="tape"] #chapter-content :is(.tp-el-heading, h2)::before {
  content: "";
  position: absolute;
  top: -.35em;
  left: 50%;
  transform: translateX(-50%) rotate(-1.7deg);
  width: 3.25em;
  height: .7em;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 30%, #ffffffbb) !important;
  border-left: 1px dashed color-mix(in srgb, var(--fx-accent, #8a5a3b) 42%, transparent);
  border-right: 1px dashed color-mix(in srgb, var(--fx-accent, #8a5a3b) 42%, transparent);
  box-shadow: 0 1px 3px #00000010;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-heading="invert"] #chapter-content :is(.tp-el-heading, h2) {
  background: var(--rdr-fg) !important;
  color: var(--rdr-bg) !important;
  padding: .55em .75em;
  text-align: left !important;
  border-bottom: .16em solid var(--fx-accent, #8a5a3b);
}

/* ==========================================================================
   2. 小节标题（data-el-subhead）
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead] #chapter-content :is(.tp-el-subhead, h3) {
  position: relative;
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead="bar"] #chapter-content :is(.tp-el-subhead, h3) {
  text-align: left !important;
  padding-left: .6em;
  border-left: .18em solid var(--fx-accent, #8a5a3b);
  line-height: 1.2 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead="pill"] #chapter-content :is(.tp-el-subhead, h3) {
  display: inline-block;
  text-align: left !important;
  font-size: .88em !important;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 12%, transparent) !important;
  color: var(--fx-accent, #8a5a3b) !important;
  padding: .17em .7em;
  border: 1px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 30%, transparent);
  border-radius: 999px;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead="centerline"] #chapter-content :is(.tp-el-subhead, h3) {
  text-align: center !important;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead="centerline"] #chapter-content :is(.tp-el-subhead, h3)::before,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-subhead="centerline"] #chapter-content :is(.tp-el-subhead, h3)::after {
  content: "";
  width: 2em;
  height: 1px;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 48%, transparent) !important;
}

/* ==========================================================================
   3. 正文首段（data-el-lead）
   ========================================================================== */
/* 首字下沉/朱印首字必须清掉段落缩进（基础规则里是 !important）。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="dropcap"] #chapter-content .tp-el-lead,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="stamp"] #chapter-content .tp-el-lead { text-indent: 0 !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="dropcap"] #chapter-content .tp-el-lead::first-letter {
  float: left;
  font-family: var(--serif) !important;
  font-size: 2.92em !important;
  line-height: .86 !important;
  padding: .06em .12em 0 0;
  color: var(--fx-accent, #8a5a3b) !important;
  font-weight: 650;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="stamp"] #chapter-content .tp-el-lead::first-letter {
  float: left;
  font-family: var(--serif) !important;
  font-size: 2.1em !important;
  line-height: 1.15 !important;
  padding: .08em .2em;
  margin: .08em .2em .05em 0;
  background: var(--fx-accent, #8a5a3b) !important;
  color: #fffdf9 !important;
  border-radius: 3px;
  box-shadow: inset 0 0 0 2px #ffffff44;
  font-weight: 600;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="symbol"] #chapter-content .tp-el-lead::before {
  content: "❖";
  display: block;
  margin-bottom: .25em;
  color: var(--fx-accent, #8a5a3b) !important;
  font-size: .8em !important;
  letter-spacing: .38em;
  text-indent: 0;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="leadline"] #chapter-content .tp-el-lead::first-line {
  font-family: var(--serif) !important;
  font-weight: 650;
  color: var(--fx-accent, #8a5a3b) !important;
  letter-spacing: .04em;
}
/* 首段以引号/括号开头时不做首字下沉：单独放大标点会遮挡正文，改为正常首字。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="dropcap"] #chapter-content .tp-el-lead-quoted::first-letter,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead="stamp"] #chapter-content .tp-el-lead-quoted::first-letter {
  float: none;
  font-size: inherit !important;
  line-height: inherit !important;
  padding: 0;
  color: inherit !important;
  background: none !important;
  border: 0;
  font-weight: inherit;
}

/* ==========================================================================
   4. 引用段落（data-el-quote）
   ========================================================================== */
/* 非「默认」时统一按原型基础引用观感重排：左右外边距归零、去掉组件基础透明度、
   内层 p 不再吃正文段落的字号/缩进 !important。各变体在此基础上覆盖。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome]:where(:not([data-el-quote="default"])) #chapter-content blockquote {
  position: relative;
  margin: 1.05em 0;
  padding: .65em .8em .65em 1em;
  border: 0;
  border-left: 2px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 58%, transparent);
  border-radius: 0;
  background: color-mix(in srgb, var(--rdr-fg) 3%, transparent) !important;
  color: color-mix(in srgb, var(--rdr-fg) 86%, transparent) !important;
  box-shadow: none;
  font-size: .88em !important;
  line-height: 1.78 !important;
  text-indent: 0;
  opacity: 1;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome]:where(:not([data-el-quote="default"])) #chapter-content blockquote :is(p, div, span, li, em, strong, b, i) {
  margin: .35em 0 !important;
  text-indent: 0 !important;
  font-size: 1em !important;
  line-height: inherit !important;
  color: inherit !important;
  background: transparent !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="bigquote"] #chapter-content blockquote {
  border: 0;
  border-left: 0;
  background: none !important;
  padding: .4em .4em .4em 2.3em;
  font-size: .96em !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="bigquote"] #chapter-content blockquote::before {
  content: "“";
  position: absolute;
  left: -.02em;
  top: -.22em;
  font-family: var(--serif) !important;
  font-size: 4.1em !important;
  line-height: 1 !important;
  color: color-mix(in srgb, var(--fx-accent, #8a5a3b) 56%, transparent) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="card"] #chapter-content blockquote {
  padding: .9em 1.05em;
  border: 1px solid color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  border-left: 1px solid color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  border-radius: 9px;
  background: color-mix(in srgb, var(--rdr-bg) 88%, #ffffff) !important;
  box-shadow: 0 4px 13px #00000008;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="center"] #chapter-content blockquote {
  padding: .6em 1.6em;
  border: 0;
  border-left: 0;
  background: none !important;
  text-align: center;
  color: color-mix(in srgb, var(--rdr-fg) 72%, transparent) !important;
  font-family: var(--serif) !important;
  font-size: .92em !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="manuscript"] #chapter-content blockquote {
  padding: .82em 1.05em;
  border: 1px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 48%, transparent);
  border-right: 4px solid var(--fx-accent, #8a5a3b);
  border-radius: 0;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 5%, transparent) !important;
  font-family: var(--serif) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="manuscript"] #chapter-content blockquote::after {
  content: "〔批注〕";
  position: absolute;
  top: -.8em;
  right: .9em;
  padding: 0 .4em;
  background: var(--rdr-bg) !important;
  color: var(--fx-accent, #8a5a3b) !important;
  font-family: var(--serif) !important;
  font-size: .85em !important;
  letter-spacing: .1em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="note"] #chapter-content blockquote {
  padding: .9em 1.05em;
  border: 0;
  border-left: 0;
  border-top: 2px dashed color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
  border-radius: 2px 2px 12px 2px;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 9%, color-mix(in srgb, var(--rdr-bg) 88%, #ffffff)) !important;
  box-shadow: 3px 5px 12px #0000000c;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-lead][data-el-quote="brutal"] #chapter-content blockquote {
  padding: .85em 1em;
  border: 2px solid var(--rdr-fg);
  border-left: 2px solid var(--rdr-fg);
  border-radius: 0;
  background: color-mix(in srgb, var(--rdr-bg) 88%, #ffffff) !important;
  box-shadow: 5px 5px 0 var(--fx-accent, #8a5a3b);
}

/* ==========================================================================
   5. 列表（data-el-list）
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dot"] #chapter-content ul { list-style: none; padding-left: .3em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dot"] #chapter-content li { position: relative; padding-left: 1.25em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dot"] #chapter-content li::before {
  content: "";
  position: absolute;
  left: .15em;
  top: .72em;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--fx-accent, #8a5a3b) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dash"] #chapter-content ul { list-style: none; padding-left: .3em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dash"] #chapter-content li { position: relative; padding-left: 1.4em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="dash"] #chapter-content li::before {
  content: "—";
  position: absolute;
  left: 0;
  color: var(--fx-accent, #8a5a3b) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="rule"] #chapter-content ul {
  list-style: none;
  margin-left: .2em;
  padding-left: .9em;
  border-left: 2px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="rule"] #chapter-content li { padding-left: .35em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="numbered"] #chapter-content ul { list-style: none; padding-left: .2em; counter-reset: item; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="numbered"] #chapter-content li { position: relative; padding-left: 2em; counter-increment: item; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="numbered"] #chapter-content li::before {
  content: counter(item, decimal-leading-zero);
  position: absolute;
  left: 0;
  top: .12em;
  color: var(--fx-accent, #8a5a3b) !important;
  font-family: var(--mono) !important;
  font-size: .52em !important;
  font-weight: 600;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="check"] #chapter-content ul { list-style: none; padding-left: .2em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="check"] #chapter-content li { position: relative; padding-left: 1.5em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="check"] #chapter-content li::before {
  content: "✓";
  position: absolute;
  left: 0;
  top: .3em;
  width: 1.05em;
  height: 1.05em;
  border: 1.5px solid var(--fx-accent, #8a5a3b);
  border-radius: 3px;
  color: var(--fx-accent, #8a5a3b) !important;
  font-family: var(--sans) !important;
  font-size: .56em !important;
  line-height: 1.05em !important;
  text-align: center;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="ornament"] #chapter-content ul { list-style: none; padding-left: .2em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="ornament"] #chapter-content li { position: relative; padding-left: 1.4em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-list="ornament"] #chapter-content li::before {
  content: "❖";
  position: absolute;
  left: 0;
  top: .2em;
  color: var(--fx-accent, #8a5a3b) !important;
  font-size: .56em !important;
}

/* ==========================================================================
   6. 插图（data-el-image）
   真实 DOM 有两层：Figure 容器（figure / p / div，均带 .tp-el-figure）与裸 <img>。
   容器变体负责画框，裸图变体负责无容器时自绘外框；容器内的 img 统一清掉自己那层，避免双重描边。
   ========================================================================== */
/* 圆角 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="rounded"] #chapter-content figure,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="rounded"] #chapter-content :is(p, div).tp-el-figure { border-radius: 14px; overflow: hidden; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="rounded"] #chapter-content img { border-radius: 14px; }
/* 白卡衬底 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="paper"] #chapter-content :is(figure, p, div).tp-el-figure {
  margin: 1.2em 0 !important;
  padding: .55em .55em 0;
  border: 1px solid color-mix(in srgb, var(--rdr-fg) 10%, transparent);
  border-radius: 10px;
  background: color-mix(in srgb, var(--rdr-bg) 88%, #ffffff) !important;
  box-shadow: 0 10px 24px #00000012;
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="paper"] #chapter-content img {
  padding: .45em;
  border: 1px solid color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--rdr-bg) 88%, #ffffff) !important;
  box-shadow: 0 9px 22px #00000012;
}
/* 通栏：翻页模式占满栏宽；滚动模式按左右边距出血到窗口边缘。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="full"] #chapter-content :is(figure, p, div).tp-el-figure {
  margin: 1.2em 0 !important;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: none !important;
  box-shadow: none;
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="full"] #chapter-content img {
  width: 100% !important;
  max-width: 100% !important;
  border: 0;
  border-radius: 0;
  padding: 0;
  background: none !important;
  box-shadow: none;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="full"] .col-scroll #chapter-content :is(figure, p, div).tp-el-figure {
  margin-left: calc(var(--rdr-mh-eff, var(--rdr-mh, 0px)) * -1) !important;
  margin-right: calc(var(--rdr-mh-eff, var(--rdr-mh, 0px)) * -1) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="full"] .col-scroll #chapter-content img {
  width: calc(100% + var(--rdr-mh-eff, var(--rdr-mh, 0px)) * 2) !important;
  max-width: none !important;
  margin-left: calc(var(--rdr-mh-eff, var(--rdr-mh, 0px)) * -1) !important;
  margin-right: calc(var(--rdr-mh-eff, var(--rdr-mh, 0px)) * -1) !important;
}
/* 内嵌框 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="fold"] #chapter-content :is(figure, p, div).tp-el-figure {
  margin: 1.2em 0 !important;
  padding: .45em;
  border: 0;
  border-radius: 4px;
  background: color-mix(in srgb, var(--rdr-fg) 5%, transparent) !important;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="fold"] #chapter-content img {
  padding: .4em;
  border: 1px solid color-mix(in srgb, var(--rdr-fg) 12%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--rdr-fg) 5%, transparent) !important;
}
/* 拍立得 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="polaroid"] #chapter-content :is(figure, p, div).tp-el-figure {
  width: 86%;
  margin: 1.5em auto !important;
  padding: .55em .55em 1.1em;
  border: 1px solid #e0d8cc;
  border-radius: 3px;
  background: #fffdf9 !important;
  color: #333 !important;
  box-shadow: 0 13px 26px #00000018;
  transform: rotate(-1.1deg);
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="polaroid"] #chapter-content img {
  padding: .5em .5em 1em;
  border: 1px solid #e0d8cc;
  border-radius: 3px;
  background: #fffdf9 !important;
  box-shadow: 0 13px 26px #00000018;
  transform: rotate(-1.1deg);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="polaroid"] #chapter-content :is(p, div, figcaption).tp-el-caption {
  color: #555 !important;
  font-family: var(--kai, var(--serif)) !important;
}
/* 复古胶片 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(figure, p, div).tp-el-figure {
  margin: 1.2em 0 !important;
  padding: .8em 0;
  border: 0;
  border-radius: 4px;
  background: #1c1a18 !important;
  box-shadow: none;
  text-indent: 0 !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(figure, p, div).tp-el-figure::before,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(figure, p, div).tp-el-figure::after {
  content: "";
  display: block;
  height: 7px;
  margin: 0 .6em;
  background: repeating-linear-gradient(90deg, #fdf5e6cc 0 8px, transparent 8px 17px) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content img {
  padding: .45em;
  border: 0;
  border-radius: 3px;
  background: #1c1a18 !important;
  filter: sepia(.12) contrast(1.04);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(p, div, figcaption).tp-el-caption { color: #b5aba0 !important; }
/* 容器内的图片：清掉裸图那一层外框，避免容器与图片各画一圈。 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome]:where(:not([data-el-image="default"])) #chapter-content :is(figure, p, div).tp-el-figure > img,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome]:where(:not([data-el-image="default"])) #chapter-content :is(figure, p, div).tp-el-figure > a > img {
  padding: 0;
  border: 0;
  border-radius: 5px;
  background: none !important;
  box-shadow: none;
  transform: none;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(figure, p, div).tp-el-figure > img,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="film"] #chapter-content :is(figure, p, div).tp-el-figure > a > img {
  margin: .8em auto;
  filter: sepia(.12) contrast(1.04);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="paper"] #chapter-content :is(figure, p, div).tp-el-figure > img,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="paper"] #chapter-content :is(figure, p, div).tp-el-figure > a > img { padding: .2em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="polaroid"] #chapter-content :is(figure, p, div).tp-el-figure > img,
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-image="polaroid"] #chapter-content :is(figure, p, div).tp-el-figure > a > img { padding: 0 0 .4em; }

/* ==========================================================================
   7. 表格（data-el-table）
   组件基础样式给每个 th/td 都画了四边 1px；不重置就会把所有变体都变成网格表。
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table]:not([data-el-table="default"]) #chapter-content table { border-collapse: collapse; }
/* 极简横线 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="line"] #chapter-content table { border: 0 !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="line"] #chapter-content :is(th, td) {
  border: 0 !important;
  border-bottom: 1px solid color-mix(in srgb, var(--rdr-fg) 14%, transparent) !important;
  padding: .5em .6em;
}
/* 三线学术 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="threeline"] #chapter-content table {
  border: 0;
  border-top: 2px solid var(--rdr-fg);
  border-bottom: 2px solid var(--rdr-fg);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="threeline"] #chapter-content :is(th, td) { border: 0; padding: .5em .6em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="threeline"] #chapter-content th {
  border-bottom: 1px solid color-mix(in srgb, var(--rdr-fg) 55%, transparent);
  color: var(--rdr-fg) !important;
}
/* 斑马纹 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="zebra"] #chapter-content table { border: 0; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="zebra"] #chapter-content :is(th, td) { border: 0; padding: .5em .6em; background: transparent !important; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="zebra"] #chapter-content tbody tr:nth-child(odd) {
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 7%, transparent) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="zebra"] #chapter-content th {
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 14%, transparent) !important;
  color: var(--fx-accent, #8a5a3b) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="zebra"] #chapter-content th { border-bottom: 1px solid color-mix(in srgb, var(--rdr-fg) 14%, transparent); }
/* 乌丝方格 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="grid"] #chapter-content table { border: 2px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 55%, transparent); }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="grid"] #chapter-content :is(th, td) {
  border: 1px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 35%, transparent);
  padding: .5em .6em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-table="grid"] #chapter-content th {
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 10%, transparent) !important;
  color: var(--fx-accent, #8a5a3b) !important;
}

/* ==========================================================================
   8. 行内链接（data-el-link）
   组件基础把 a 设成继承色 + 无下划线；原型里链接统一用强调色，各变体只换装饰。
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="underline"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border-bottom: 0;
  text-decoration: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: .18em;
  text-decoration-color: color-mix(in srgb, var(--fx-accent, #8a5a3b) 55%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="arrow"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border-bottom: 0;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="arrow"] #chapter-content a:not(.tp-el-heading-link)::after {
  content: " ↗";
  font-size: .78em !important;
  font-family: var(--sans) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="highlight"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border-bottom: 0;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 18%, transparent) !important;
  padding: 0 .2em;
  border-radius: 4px;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="color"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border-bottom: 0;
  font-weight: 600;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="dashed"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border-bottom: 1.5px dashed var(--fx-accent, #8a5a3b);
  padding-bottom: .08em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-link="badge"] #chapter-content a:not(.tp-el-heading-link) {
  color: var(--fx-accent, #8a5a3b) !important;
  border: 1px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
  border-radius: 999px;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 9%, transparent) !important;
  padding: .02em .4em;
  font-size: .88em !important;
}

/* ==========================================================================
   9. 重点强调（data-el-em）
   应用真实 DOM 是 em/strong/b/i（原型为 mark.fx），全部按同一套效果处理。
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="mark"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) {
  background: linear-gradient(transparent 58%, color-mix(in srgb, var(--fx-accent, #8a5a3b) 34%, transparent) 58%) !important;
  padding: 0 .05em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="underline"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) {
  background: none !important;
  border-bottom: 2.5px solid var(--fx-accent, #8a5a3b);
  padding: 0 .05em;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="color"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) {
  background: none !important;
  color: var(--fx-accent, #8a5a3b) !important;
  font-weight: 600;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="bracket"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) { background: none; padding: 0; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="bracket"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *)::before { content: "「"; color: var(--fx-accent, #8a5a3b); }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="bracket"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *)::after { content: "」"; color: var(--fx-accent, #8a5a3b); }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="dot"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) {
  background: none !important;
  text-emphasis: filled circle var(--fx-accent, #8a5a3b);
  -webkit-text-emphasis: filled circle var(--fx-accent, #8a5a3b);
  text-emphasis-position: under right;
  -webkit-text-emphasis-position: under right;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-em="circle"] #chapter-content :is(em, strong, b, i, mark.fx):not(:is(.tp-el-heading, .tp-el-subhead) *) {
  background: none !important;
  border: 1.5px solid var(--fx-accent, #8a5a3b);
  border-radius: 999px;
  padding: .02em .35em;
  color: var(--fx-accent, #8a5a3b) !important;
}

/* ==========================================================================
   10. 章节分隔（data-el-hr）
   场景分割线在 app 里是只含符号的 p/div（.tp-el-hr），也可能来自原书 <hr>。
   选中非默认样式后先隐藏原符号，再由下面的规则统一接管。
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome]:where(:not([data-el-hr="default"])) #chapter-content :is(p, div).tp-el-hr {
  font-size: 0 !important;
  line-height: 1.6 !important;
  text-indent: 0 !important;
  margin: 1.6em auto !important;
  padding: 0;
  border: 0;
  background: none !important;
  min-height: 0;
}
/* 细线 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="line"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: block;
  width: 100%;
  height: 0;
  min-height: 0;
  margin: 1.6em auto !important;
  padding: 0;
  border: 0;
  border-top: 1px solid color-mix(in srgb, var(--rdr-fg) 20%, transparent);
  background: none !important;
  overflow: visible;
}
/* 三点 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="dots"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: block;
  width: 100%;
  height: auto;
  min-height: 0;
  margin: 1.6em auto !important;
  padding: 0;
  border: 0;
  background: none !important;
  text-align: center;
  overflow: visible;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="dots"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr)::before {
  content: "· · ·";
  color: color-mix(in srgb, var(--rdr-fg) 42%, transparent) !important;
  font-family: var(--serif) !important;
  font-size: calc(var(--rdr-fs, 32px) * .8) !important;
  letter-spacing: .35em;
}
/* 单菱形 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="diamond"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: block;
  width: 100%;
  height: calc(var(--rdr-fs, 32px) * 1.2);
  min-height: 0;
  margin: 1.5em auto !important;
  padding: 0;
  border: 0;
  background: none !important;
  text-align: center;
  overflow: visible;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="diamond"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr)::before {
  content: "◆";
  color: var(--fx-accent, #8a5a3b) !important;
  opacity: .65;
  font-size: calc(var(--rdr-fs, 32px) * .42) !important;
}
/* 卷轴花纹 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="scroll"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  width: 100%;
  height: auto;
  min-height: 0;
  margin: 1.6em auto !important;
  padding: 0;
  border: 0;
  background: none !important;
  overflow: visible;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="scroll"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr)::before {
  content: "───  ❖  ───";
  color: var(--fx-accent, #8a5a3b) !important;
  font-family: var(--serif) !important;
  font-size: calc(var(--rdr-fs, 32px) * .48) !important;
  letter-spacing: .2em;
  opacity: .78;
}
/* 纯留白 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="space"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: block;
  width: 100%;
  height: calc(var(--rdr-fs, 32px) * 1.5);
  min-height: 0;
  margin: 1em auto !important;
  padding: 0;
  border: 0;
  background: none !important;
}
/* 裁切线 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="cut"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr) {
  display: block;
  position: relative;
  width: 100%;
  height: 0;
  min-height: 0;
  margin: 1.8em auto !important;
  padding: 0;
  border: 0;
  border-top: 1.5px dashed color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
  background: none !important;
  overflow: visible;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-hr="cut"] #chapter-content :is(p.tp-el-hr, div.tp-el-hr, hr)::before {
  content: "✂";
  position: absolute;
  left: 16px;
  top: -.8em;
  padding: 0 .45em;
  background: var(--rdr-bg) !important;
  color: var(--fx-accent, #8a5a3b) !important;
  font-size: calc(var(--rdr-fs, 32px) * .4) !important;
}

/* ==========================================================================
   11. 页眉页脚（data-el-chrome）
   应用 DOM：.topbar（.btitle / .stat-item / .stat-sep）+ .footer（.finfo / .fitem / .fsep）。
   原型里的 .pill 对应顶栏中部的状态项；底部分隔线在组件里由 .topbar::after 画。
   ========================================================================== */
/* 书卷双线 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="classic"] .topbar { font-family: var(--serif); }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="classic"] .topbar :is(.btitle, .stat-item, .stat-sep) { font-size: 11px; letter-spacing: .16em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="classic"] .topbar::after {
  height: 0;
  background: none !important;
  border-top: 3px double color-mix(in srgb, var(--rdr-fg) 22%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="classic"] .footer { font-family: var(--serif); letter-spacing: .12em; }
/* 杂志粗线 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="magazine"] .topbar { text-transform: uppercase; font-weight: 700; letter-spacing: .14em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="magazine"] .topbar::after {
  height: 0;
  background: none !important;
  border-top: 2px solid var(--rdr-fg);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="magazine"] .topbar .stat-item {
  padding: 2px 6px;
  border: 1px solid var(--fx-accent, #8a5a3b);
  border-radius: 0;
  background: var(--fx-accent, #8a5a3b) !important;
  color: #fff !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="magazine"] .footer { font-family: var(--sans); font-size: 9px; letter-spacing: .14em; }
/* 极简隐匿 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="minimal"] .topbar:not(.dim) { opacity: .55; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="minimal"] .topbar::after { display: none; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="minimal"] .topbar :is(.stat-item, .stat-sep) { display: none; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="minimal"] .footer:not(.dim) { opacity: .5; font-size: 9px; }
/* 线装书耳 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="guji"] .topbar {
  font-family: var(--serif) !important;
  letter-spacing: .22em;
  background: color-mix(in srgb, var(--fx-accent, #8a5a3b) 4%, var(--rdr-bg)) !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="guji"] .topbar::after {
  height: 0;
  background: none !important;
  border-top: 2px solid color-mix(in srgb, var(--fx-accent, #8a5a3b) 45%, transparent);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="guji"] .topbar .stat-item {
  padding: 1px 5px;
  border: 1px solid var(--fx-accent, #8a5a3b);
  border-radius: 2px;
  color: var(--fx-accent, #8a5a3b) !important;
}
/* 手账网格 */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="journal"] .topbar {
  background-image: radial-gradient(color-mix(in srgb, var(--fx-accent, #8a5a3b) 28%, transparent) 1px, transparent 1px) !important;
  background-size: 10px 10px !important;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-chrome="journal"] .topbar::after {
  height: 0;
  background: none !important;
  border-top: 1.5px dashed color-mix(in srgb, var(--fx-accent, #8a5a3b) 40%, transparent);
}

/* ==========================================================================
   12. 脚注浮层变体（data-el-footnote）
   ========================================================================== */
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="paper"] .tp-fn-pop {
  border-color: #d9c8b4;
  border-radius: 7px;
  background: #fffaf2 !important;
  color: #2b2520 !important;
  box-shadow: 0 14px 36px #4a352021;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="paper"] .tp-fn-head { color: #8a5a3b; border-bottom-color: #eaded0; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="seal"] .tp-fn-pop {
  border-left: 3px solid var(--fx-accent, #8a5a3b);
  border-radius: 3px;
  background: color-mix(in srgb, #fff8ee 90%, var(--fx-accent, #8a5a3b) 10%) !important;
  color: #2b2520 !important;
  box-shadow: 0 14px 36px #4a352021;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="seal"] .tp-fn-head { color: var(--fx-accent, #8a5a3b); letter-spacing: .2em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="minimal"] .tp-fn-pop {
  border-color: color-mix(in srgb, var(--rdr-fg) 14%, transparent);
  border-radius: 5px;
  background: color-mix(in srgb, var(--rdr-bg) 94%, transparent) !important;
  box-shadow: 0 8px 26px #1c17121a;
  backdrop-filter: blur(8px);
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="minimal"] .tp-fn-head { padding-top: 6px; padding-bottom: 5px; letter-spacing: .1em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="minimal"] .tp-fn-body { padding: 8px 12px 11px; font-size: .95em; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="ink"] .tp-fn-pop {
  border-color: #55483c;
  border-radius: 6px;
  background: #2b2520 !important;
  color: #f5ede2 !important;
  box-shadow: 0 18px 52px #0003;
}
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="ink"] .tp-fn-head { color: #cbbba7; border-bottom-color: #55483c; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="ink"] .tp-fn-body a { color: #e3a47f; }
.reader[data-el-image][data-el-footnote][data-el-chrome][data-el-table][data-el-footnote="ink"] .tp-fn-head .tp-fn-close { color: #cbbba7; }
`;
