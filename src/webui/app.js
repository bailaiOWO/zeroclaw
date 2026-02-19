const pageTitles = {
  dashboard:'仪表盘', chat:'对话', channels:'消息平台',
  providers:'模型与路由', config:'更多配置', settings:'设置', 
  identity:'身份设定', cron:'定时任务',
  'context-files':'上下文文件'
};
let currentTab = 'dashboard';
let contextAutoRefreshEnabled = false;
let contextAutoRefreshTimer = null;

function switchTab(name){
  const previous = currentTab;
  currentTab = name;
  document.querySelectorAll('.nav-item').forEach(btn=>{
    btn.classList.toggle('active', btn.getAttribute('onclick').includes("'"+name+"'"));
  });
  document.querySelectorAll('.page').forEach(p=>p.classList.remove('active'));
  const page = document.getElementById('page-'+name);
  if(page) page.classList.add('active');
  const cc = document.getElementById('chat-container');
  if(cc) cc.classList.toggle('active', name==='chat');
  document.getElementById('page-title').textContent = pageTitles[name]||name;

  if(previous==='context-files' && name!=='context-files') stopContextAutoRefresh();

  if(['channels','providers','config'].includes(name)) loadConfig();
  if(name==='identity') loadIdentity();
  if(name==='cron') loadCron();
  if(name==='context-files'){
    loadContextFiles();
    if(contextAutoRefreshEnabled) startContextAutoRefresh();
  }
}

function refreshCurrent(){
  loadStatus();
  if(['channels','providers','config'].includes(currentTab)) loadConfig();
  if(currentTab==='identity') loadIdentity();
  if(currentTab==='cron') loadCron();
  if(currentTab==='context-files') loadContextFiles();
}

function fmtUptime(s){
  if(s==null) return '—';
  const d=Math.floor(s/86400),h=Math.floor(s%86400/3600),m=Math.floor(s%3600/60);
  let r='';
  if(d) r+=d+'天 ';
  if(h) r+=h+'时 ';
  r+=m+'分';
  return r;
}
function boolZh(v){return v?'已启用':'已禁用'}
function boolCls(v){return v?'val-true':'val-false'}
function esc(s){const d=document.createElement('div');d.textContent=s;return d.innerHTML}
function stripCustom(s){ return (s||'').replace(/^custom:/, ''); }
function toggleExpand(el){
  const p = el.closest('.expandable-item');
  p.classList.toggle('expanded');
}
function arr2str(arr){ return (arr||[]).join(', '); }
function str2arr(str){ return (str||'').split(',').map(s=>s.trim()).filter(Boolean); }

let _rawConfig = null;
let _providerModelCandidates = {};
let _providerModelStatus = {};
let currentContextFile = '';

async function loadStatus(){
  try{
    const r=await fetch('/api/status');
    if(!r.ok) throw new Error();
    const d=await r.json();

    document.getElementById('status-headline').textContent='v'+d.version+' · 运行 '+fmtUptime(d.uptime_seconds);
    document.getElementById('v-version').textContent=d.version||'—';
    document.getElementById('v-uptime').textContent=fmtUptime(d.uptime_seconds);
    document.getElementById('v-pid').textContent=d.pid??'—';
    document.getElementById('v-provider').textContent=stripCustom(d.provider)||'—';
    document.getElementById('v-model').textContent=d.model||'—';
    document.getElementById('v-temp').textContent=d.temperature??'—';
    document.getElementById('v-config-model').textContent=d.configured_model||'—';
    const modelAlert = document.getElementById('v-model-alert');
    if(modelAlert){
      const needsRestart = !!d.model_needs_restart;
      modelAlert.classList.toggle('is-hidden', !needsRestart);
      if(needsRestart){
        modelAlert.textContent = `检测到运行模型(${d.model||'—'}) 与配置模型(${d.configured_model||'—'}) 不一致：重启后才会完全生效。`;
      }
    }
    document.getElementById('v-autonomy').textContent=d.autonomy_level||'—';
    document.getElementById('v-memory').textContent=d.memory_backend||'—';
    document.getElementById('v-autosave').textContent=d.auto_save?'已启用':'已禁用';
    document.getElementById('v-runtime').textContent=d.runtime||'—';
    document.getElementById('v-observability').textContent=d.observability||'—';
    document.getElementById('v-tunnel').textContent=d.tunnel||'none';
    document.getElementById('v-paired').textContent=d.paired?'已配对':'未配对';

    const ch=d.channels||{};
    const names={cli:'CLI',telegram:'Telegram',discord:'Discord',slack:'Slack',webhook:'Webhook',whatsapp:'WhatsApp'};
    const icons={cli:'terminal',telegram:'send',discord:'headset_mic',slack:'tag',webhook:'webhook',whatsapp:'chat'};
    const box=document.getElementById('channels-chips');
    box.innerHTML='';
    for(const[k,label] of Object.entries(names)){
      const on=!!ch[k];
      const chip=document.createElement('span');
      chip.className='chip '+(on?'enabled':'disabled');
      chip.innerHTML='<span class="material-symbols-outlined">'+(icons[k]||'circle')+'</span>'+label;
      box.appendChild(chip);
    }
  }catch(e){
    document.getElementById('status-headline').textContent='无法连接';
  }
}

async function loadConfig(){
  try{
    const r=await fetch('/api/config/raw');
    if(!r.ok) throw new Error();
    _rawConfig = await r.json();
    renderChannels(_rawConfig);
    renderProviders(_rawConfig);
    renderRawConfig(_rawConfig);
  }catch(e){}
}

async function saveRawConfig(){
  flushFormsToConfig();
  const btn=event.target; const old=btn.textContent;
  btn.textContent='保存中…';btn.disabled=true;
  try{
    const r=await fetch('/api/config/raw',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(_rawConfig)});
    if(r.ok){
        const resp = await r.json();
        if(resp.requires_restart) alert('配置保存成功！\n\n【注意】部分渠道或核心配置的修改，需要重新启动 ZeroClaw 进程才能生效。');
        else alert('配置保存成功！');
        loadStatus(); 
    }
    else alert('保存失败:'+await r.text());
  }catch(e){alert('网络错误')}
  finally{btn.textContent=old;btn.disabled=false;}
}

const ALL_CHANNELS = [
  {key:'telegram', name:'Telegram', icon:'send', desc:'Telegram Bot', schema:{bot_token:'str',allowed_users:'arr'}},
  {key:'discord',  name:'Discord', icon:'headset_mic', desc:'Discord Bot', schema:{bot_token:'str',guild_id:'str',allowed_users:'arr',listen_to_bots:'bool',mention_only:'bool'}},
  {key:'slack',    name:'Slack', icon:'tag', desc:'Slack Bot', schema:{bot_token:'str',app_token:'str',channel_id:'str',allowed_users:'arr'}},
  {key:'qq',       name:'QQ Official', icon:'smart_toy', desc:'QQ 官方机器人', schema:{app_id:'str',app_secret:'str',allowed_users:'arr'}},
  {key:'webhook',  name:'Webhook', icon:'webhook', desc:'HTTP 回调接口', schema:{port:'num',secret:'str'}},
  {key:'whatsapp', name:'WhatsApp', icon:'chat', desc:'Meta Business API', schema:{access_token:'str',phone_number_id:'str',verify_token:'str',app_secret:'str',allowed_numbers:'arr'}},
  {key:'lark',     name:'飞书 / Lark', icon:'apartment', desc:'飞书开放平台', schema:{app_id:'str',app_secret:'str',encrypt_key:'str',verification_token:'str',allowed_users:'arr',use_feishu:'bool',receive_mode:'str',port:'num'}},
  {key:'dingtalk', name:'钉钉 DingTalk', icon:'campaign', desc:'钉钉 Stream 模式', schema:{client_id:'str',client_secret:'str',allowed_users:'arr'}},
  {key:'cli',      name:'CLI 终端', icon:'terminal', desc:'启用命令行交互通道', schema:{}},
];

const CH_DICT = {bot_token:'机器人 Token', allowed_users:'授权用户ID (逗号分隔)', guild_id:'服务器 ID (Guild)', listen_to_bots:'监听其他机器人', mention_only:'仅响应@提及', app_token:'App Token', channel_id:'频道 ID', app_id:'应用 ID', app_secret:'应用密钥', port:'监听端口', secret:'密钥 (可选)', access_token:'访问令牌', phone_number_id:'电话号码ID', verify_token:'验证令牌', allowed_numbers:'授权电话号码', encrypt_key:'加密密钥', verification_token:'事件订阅Token', use_feishu:'使用飞书(而非Lark)', receive_mode:'接收模式(websocket/webhook)', client_id:'Client ID', client_secret:'Client Secret'};


function renderChannels(cfg){
  const chs = cfg.channels_config||{};
  const box=document.getElementById('channels-list');
  box.innerHTML='';
  for(const c of ALL_CHANNELS){
    let data = chs[c.key];
    const isOn = c.key==='cli' ? (chs.cli===true) : !!data;
    if(!data) data={};

    const el=document.createElement('div');
    el.className='expandable-item'+(isOn?' active-item':'');
    el.innerHTML=`
      <div class="expandable-header" onclick="toggleExpand(this)">
        <div class="expandable-icon"><span class="material-symbols-outlined">${c.icon}</span></div>
        <div class="expandable-main">
          <div class="expandable-title">${esc(c.name)}</div>
          <div class="expandable-subtitle">${esc(c.desc)}</div>
        </div>
        <div class="switch" onclick="event.stopPropagation()">
          <input type="checkbox" ${isOn?'checked':''} 
             onchange="window.toggleChannel(this, '${c.key}', this.checked)">
          <span class="switch-slider"></span>
        </div>
        <span class="material-symbols-outlined chevron-icon">expand_more</span>
      </div>
      <div class="expandable-body-wrapper">
        <div class="expandable-body">
          <div class="expandable-body-inner config-grid" id="body-ch-${c.key}"></div>
        </div>
      </div>
    `;
    box.appendChild(el);

    const b = el.querySelector('.expandable-body-inner');
    if(Object.keys(c.schema).length===0) b.innerHTML='<div class="form-empty">无需额外配置。</div>';
    for(const [k, type] of Object.entries(c.schema)){
      const fg=document.createElement('div');fg.className='form-group';
      let val = data[k];
      const label = CH_DICT[k] || k;
      if(type==='arr') val = arr2str(val);
      if(type==='bool'){
        fg.innerHTML=`<label class="checkbox-field"><input type="checkbox" data-path="channels_config.${c.key}.${k}" ${val?'checked':''}> <span>${esc(label)}</span></label>`;
      }else{
        fg.innerHTML=`<label>${esc(label)}</label><input class="form-input" type="${type==='num'?'number':'text'}" data-path="channels_config.${c.key}.${k}" data-valtype="${type}" value="${esc(val??'')}">`;
      }
      b.appendChild(fg);
    }
  }
}

window.toggleChannel = function(el, key, checked){
  if(key==='cli'){
    _rawConfig.channels_config.cli = checked;
  }else{
    if(checked) {
      if(!_rawConfig.channels_config[key]) {
         _rawConfig.channels_config[key] = {};
         if(key === 'lark') {
             _rawConfig.channels_config.lark.receive_mode = 'websocket';
             _rawConfig.channels_config.lark.port = 8080;
         }
         if(key === 'webhook') {
             _rawConfig.channels_config.webhook.port = 8080;
         }
      }
    } else {
      _rawConfig.channels_config[key] = null;
    }
  }
  // Visually toggle parent styling instead of re-rendering everything to preserve open state
  const item = el.closest('.expandable-item');
  if(checked) item.classList.add('active-item');
  else item.classList.remove('active-item');
};

const ALL_PROVIDERS = [
  {id:'openrouter', name:'OpenRouter'},
  {id:'anthropic',  name:'Anthropic'},
  {id:'openai',     name:'OpenAI'},
  {id:'gemini',     name:'Google Gemini'},
  {id:'ollama',     name:'Ollama'},
  {id:'glm',        name:'智谱 GLM / Z.AI'},
  {id:'minimax',    name:'MiniMax'},
  {id:'moonshot',   name:'Moonshot / Kimi'},
  {id:'qwen',       name:'通义千问 Qwen'},
  {id:'zai',        name:'Z.AI Coding'},
  {id:'compatible', name:'自定义兼容 (Custom)'},
];

function addCustomProvider() {
  const p = prompt('请输入自定义提供商的标识 (例如 custom:https://api.deepseek.com/v1)');
  if (p && p.trim()) {
    _rawConfig.default_provider = p.trim();
    renderProviders(_rawConfig);
  }
}

function renderProviders(cfg){
  let rawProvider = cfg.default_provider || '';
  let isCustom = rawProvider.startsWith('custom:');
  let currentId = stripCustom(rawProvider).toLowerCase();
  
  const compatibleUrlValue = rawProvider.startsWith('custom:') ? stripCustom(rawProvider) : (cfg.api_url||'');

  const displayProviders = [...ALL_PROVIDERS];
  if (rawProvider && !rawProvider.startsWith('custom:') && !ALL_PROVIDERS.find(p => p.id === currentId || p.id === rawProvider)) {
     displayProviders.unshift({ id: rawProvider, name: isCustom ? rawProvider : '自定义: ' + rawProvider });
  }

  const box=document.getElementById('providers-list');
  box.innerHTML='';
  for(const p of displayProviders){
    const discovered = _providerModelCandidates[p.id] || [];
    const discoverStatus = _providerModelStatus[p.id] || '';

    const isActive = currentId === p.id
      || rawProvider === p.id
      || currentId.startsWith(p.id)
      || (p.id==='compatible' && rawProvider.startsWith('custom:'));
    const el=document.createElement('div');
    el.className='expandable-item'+(isActive?' active-item':'');
    el.innerHTML=`
      <div class="expandable-header" onclick="toggleExpand(this)">
        <div class="expandable-icon"><span class="material-symbols-outlined">dns</span></div>
        <div class="expandable-main">
          <div class="expandable-title">${esc(p.name)} ${isActive?'<span class="provider-active-tag">默认激活</span>':''}</div>
        </div>
        ${!isActive?`<button class="btn btn-text btn-sm" onclick="event.stopPropagation();setDefaultProvider('${p.id}')">设为全局默认</button>`:''}
        <span class="material-symbols-outlined chevron-icon">expand_more</span>
      </div>
      <div class="expandable-body-wrapper">
        <div class="expandable-body">
          <div class="expandable-body-inner config-grid">
            <div class="form-group">
              <label>默认模型（Model）</label>
              <div class="inline-field provider-model-row">
                <input class="form-input" type="text" id="pv-mod-${p.id}" ${isActive?'data-path="default_model"':''} value="${isActive?esc(cfg.default_model||''):''}" placeholder="填入具体模型代号">
                <button class="btn btn-text btn-sm" id="pv-discover-btn-${p.id}" onclick="event.stopPropagation();discoverProviderModels('${p.id}')">从 URL 自动拉取</button>
              </div>
              <div class="field-hint" id="pv-discover-status-${p.id}">${esc(discoverStatus)}</div>
              ${discovered.length?`
              <select class="form-input provider-model-select" id="pv-discover-select-${p.id}" onchange="applyDiscoveredModel('${p.id}', this.value)">
                <option value="">选择已发现模型 (${discovered.length})…</option>
                ${discovered.map(m=>`<option value="${esc(m)}">${esc(m)}</option>`).join('')}
              </select>
              `:''}
            </div>

            <div class="form-group">
               <label>API Key / 鉴权令牌</label>
               <input class="form-input" type="password" id="pv-key-${p.id}" ${isActive?'data-path="api_key"':''} placeholder="${isActive?(cfg.api_key?'•••••••• (已配置)':'未配置'):'激活后生效'}">
            </div>

            ${p.id==='compatible'?`
            <div class="form-group">
               <label>自定义 API Base URL</label>
               <input class="form-input" type="text" id="pv-url-${p.id}" ${isActive?'data-path="api_url"':''} value="${isActive?esc(compatibleUrlValue||''):''}" placeholder="例如: https://api.openai.com/v1">
            </div>
            `:''}

            ${isActive?`<div class="field-hint full-span">提示：自动拉取会优先使用当前输入框里的 URL / API Key；为空时回退到已保存配置。</div>`:''}
          </div>
        </div>
      </div>
    `;
    box.appendChild(el);
  }

  const mb=document.getElementById('model-routes-list');
  mb.innerHTML='';
  const routes = cfg.model_routes||[];
  for(let i=0; i<routes.length; i++){
    const r = routes[i];
    const el=document.createElement('div');
    el.className='model-route-row';
    el.innerHTML=`
      <input class="form-input" placeholder="Hint (任务)" data-path="model_routes.${i}.hint" value="${esc(r.hint)}">
      <input class="form-input" placeholder="Provider (渠道)" data-path="model_routes.${i}.provider" value="${esc(r.provider)}">
      <input class="form-input" placeholder="Model (模型)" data-path="model_routes.${i}.model" value="${esc(r.model)}">
      <button class="btn btn-error btn-sm" onclick="_rawConfig.model_routes.splice(${i},1);renderProviders(_rawConfig)">删除</button>
    `;
    mb.appendChild(el);
  }
}

window.setDefaultProvider = function(id){
  if(id==='compatible'){
    const url = (document.getElementById('pv-url-compatible')?.value || _rawConfig.api_url || '').trim();
    _rawConfig.default_provider = url ? `custom:${url}` : 'custom:';
    if(url) _rawConfig.api_url = url;
  }
  else {
    _rawConfig.default_provider = id;
  }
  const mod = document.getElementById('pv-mod-'+id)?.value;
  if(mod) _rawConfig.default_model = mod;
  const k = document.getElementById('pv-key-'+id)?.value;
  if(k) _rawConfig.api_key = k;
  renderProviders(_rawConfig);
};

function resolveProviderForDiscovery(providerId){
  const raw = (_rawConfig?.default_provider||'').trim();
  if(!raw) return providerId;
  if(providerId===raw) return raw;
  if(providerId==='compatible'){
    if(raw.startsWith('custom:')) return raw;
    return providerId;
  }
  if(stripCustom(raw).toLowerCase()===providerId && raw.startsWith('custom:')) return raw;
  return providerId;
}

window.discoverProviderModels = async function(providerId){
  if(!_rawConfig) return;
  flushFormsToConfig();
  const btn = document.getElementById('pv-discover-btn-'+providerId);
  const old = btn ? btn.textContent : '';
  if(btn){ btn.disabled = true; btn.textContent = '拉取中…'; }
  try{
    const payload = { provider: resolveProviderForDiscovery(providerId) };
    const apiUrl = (document.getElementById('pv-url-'+providerId)?.value || _rawConfig.api_url || '').trim();
    const apiKey = (document.getElementById('pv-key-'+providerId)?.value || _rawConfig.api_key || '').trim();
    if(apiUrl) payload.api_url = apiUrl;
    if(apiKey) payload.api_key = apiKey;

    const r = await fetch('/api/models/discover',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'自动拉取失败');

    const models = Array.isArray(d.models) ? d.models : [];
    _providerModelCandidates[providerId] = models;
    _providerModelStatus[providerId] = models.length
      ? `已从 ${d.endpoint||'接口'} 拉取 ${models.length} 个模型`
      : '接口可达，但未返回模型列表';
    renderProviders(_rawConfig);
  }catch(e){
    _providerModelStatus[providerId] = `拉取失败：${e.message||e}`;
    renderProviders(_rawConfig);
  }finally{
    if(btn){ btn.disabled = false; btn.textContent = old; }
  }
};

window.applyDiscoveredModel = function(providerId, model){
  if(!model) return;
  const input = document.getElementById('pv-mod-'+providerId);
  if(input) input.value = model;
  if(input?.getAttribute('data-path')==='default_model') _rawConfig.default_model = model;
};

window.addModelRoute = function(){
  if(!_rawConfig.model_routes) _rawConfig.model_routes=[];
  _rawConfig.model_routes.push({hint:'',provider:'',model:''});
  renderProviders(_rawConfig);
};

function renderRawConfig(cfg){
  const box=document.getElementById('raw-config-forms');
  box.innerHTML='';
  const sections = [
    {key:'agent', name:'智能体 (Agent)', icon:'psychology'},
    {key:'autonomy', name:'自主权与沙盒', icon:'shield'},
    {key:'memory', name:'记忆系统', icon:'storage'},
    {key:'runtime', name:'执行运行时', icon:'terminal'},
    {key:'gateway', name:'网关与安全', icon:'router'},
    {key:'reliability', name:'可靠性与退避', icon:'health_and_safety'},
    {key:'scheduler', name:'任务调度', icon:'event'},
    {key:'tunnel', name:'内网穿透', icon:'vpn_key'},
    {key:'browser', name:'浏览器自动化', icon:'public'},
    {key:'observability', name:'可观测性', icon:'monitoring'}
  ];

  const dict = {
    compact_context: '紧凑上下文 (节省Token)',
    max_tool_iterations: '最大工具迭代次数',
    max_history_messages: '最大历史消息数',
    parallel_tools: '并行执行工具',
    tool_dispatcher: '工具调度器 (auto/xml)',
    level: '自主权等级',
    workspace_only: '仅限工作区读写',
    allowed_commands: '允许的系统命令',
    forbidden_paths: '禁止访问的路径',
    max_actions_per_hour: '每小时最大动作数',
    max_cost_per_day_cents: '每日最大消费 (美分)',
    require_approval_for_medium_risk: '中等风险需审批',
    block_high_risk_commands: '禁止高风险命令',
    auto_approve: '自动批准的工具',
    always_ask: '总是询问的工具',
    backend: '后端类型',
    auto_save: '自动保存',
    hygiene_enabled: '启用定期清理',
    archive_after_days: '归档阈值 (天)',
    purge_after_days: '清除阈值 (天)',
    conversation_retention_days: '对话保留 (天)',
    embedding_provider: '向量嵌入提供商',
    embedding_model: '向量嵌入模型',
    embedding_dimensions: '向量维度',
    response_cache_enabled: '启用响应缓存',
    response_cache_ttl_minutes: '响应缓存TTL (分钟)',
    snapshot_enabled: '启用记忆快照',
    auto_hydrate: '自动恢复记忆',
    kind: '运行时类型 (native/docker)',
    port: '监听端口',
    host: '监听主机',
    require_pairing: '强制配对认证',
    allow_public_bind: '允许公开绑定',
    pair_rate_limit_per_minute: '配对速率限制/分',
    webhook_rate_limit_per_minute: 'Webhook速率限制/分',
    trust_forwarded_headers: '信任代理头 (X-Forwarded-For)',
    provider_retries: '接口重试次数',
    provider_backoff_ms: '重试退避 (毫秒)',
    fallback_providers: '备用提供商链',
    channel_initial_backoff_secs: '通道初始重连退避',
    channel_max_backoff_secs: '通道最大重连退避',
    scheduler_poll_secs: '调度器轮询间隔',
    scheduler_retries: '调度器失败重试',
    enabled: '已启用',
    max_tasks: '最大任务数',
    max_concurrent: '最大并发',
    provider: '提供商/协议',
    native_headless: '无头模式 (隐藏窗口)',
    allowed_domains: '允许访问的域名'
  };

  for(const sec of sections){
    const obj = cfg[sec.key] || {};
    const el = document.createElement('div');
    el.className='expandable-item config-item';
    el.innerHTML=`
      <div class="expandable-header" onclick="toggleExpand(this)">
        <div class="expandable-icon"><span class="material-symbols-outlined">${sec.icon}</span></div>
        <div class="expandable-title">${sec.name} <span class="section-key">[${sec.key}]</span></div>
        <span class="material-symbols-outlined chevron-icon">expand_more</span>
      </div>
      <div class="expandable-body-wrapper">
        <div class="expandable-body">
          <div class="expandable-body-inner config-grid">
            ${Object.keys(obj).filter(k=>typeof obj[k]!=='object'||Array.isArray(obj[k])).map(k=>{
              const val = obj[k];
              const labelZh = dict[k] || k;
              if(typeof val === 'boolean'){
                return '<label class="checkbox-field">'+
                  '<input type="checkbox" data-path="'+sec.key+'.'+k+'" '+(val?'checked':'')+'> '+
                  '<span>'+labelZh+'</span></label>';
              }
              const isArr = Array.isArray(val);
              return '<div class="form-group">'+
                '<label>'+labelZh+(isArr?' (逗号分隔)':'')+'</label>'+
                '<input class="form-input" type="text" data-path="'+sec.key+'.'+k+'" data-valtype="'+(isArr?'arr':'')+'" value="'+esc(isArr?arr2str(val):(val??''))+'">'+
              '</div>';
            }).join('')}
          </div>
        </div>
      </div>
    `;
    box.appendChild(el);
  }
}

function setPath(obj, path, value) {
    const keys = path.split('.');
    let curr = obj;
    for (let i = 0; i < keys.length - 1; i++) {
        if (!curr[keys[i]]) curr[keys[i]] = {};
        curr = curr[keys[i]];
    }
    const last = keys[keys.length - 1];
    if (Array.isArray(curr[last]) && typeof value === 'string') curr[last] = str2arr(value);
    else if(typeof curr[last] === 'number' && typeof value === 'string') curr[last] = Number(value)||0;
    else curr[last] = value;
}

function flushFormsToConfig() {
  if(!_rawConfig) return;
  document.querySelectorAll('[data-path]').forEach(el => {
    const path = el.getAttribute('data-path');
    const vtype = el.getAttribute('data-valtype');
    const pathStr = String(path);

    if (pathStr.startsWith('channels_config.')) {
        const chKey = pathStr.split('.')[1];
        if (chKey !== 'cli' && !_rawConfig.channels_config[chKey]) return;
    }

    // Password fields (e.g. provider api_key) are intentionally rendered empty
    // and only show placeholder text like "•••• 已配置".
    // If we blindly write empty string back, saving unrelated sections (channels,
    // settings, etc.) will erase existing secrets from config.toml.
    // Therefore: empty password input means "keep current value".
    if (el.type === 'password' && !el.value) return;

    if(el.type === 'checkbox') setPath(_rawConfig, path, el.checked);
    else if(vtype === 'arr') setPath(_rawConfig, path, str2arr(el.value));
    else if(vtype === 'num') setPath(_rawConfig, path, Number(el.value)||0);
    else {
        let finalVal = el.value;
        if(pathStr === 'channels_config.lark.receive_mode' && !finalVal) finalVal = 'websocket';
        if(pathStr === 'channels_config.lark.use_feishu' && finalVal === '') finalVal = false;
        setPath(_rawConfig, path, finalVal);
    }
  });
}

let currentIdentityFile = '';
let activeIdentityFile = '';

async function loadIdentity(){
  try{
    const url = currentIdentityFile
      ? `/api/identity?file=${encodeURIComponent(currentIdentityFile)}`
      : '/api/identity';
    const r=await fetch(url);
    if(!r.ok) return;
    const d=await r.json();
    activeIdentityFile = d.active_file;
    // If we haven't picked one yet, use active or first file
    if(!currentIdentityFile){
      currentIdentityFile = d.active_file || (d.files.length ? d.files[0] : '');
    }
    document.getElementById('identity-title-disp').textContent = currentIdentityFile || '(无身份文件)';
    document.getElementById('identity-input').value = d.content || '';

    const btnAct = document.getElementById('btn-activate-identity');
    if(currentIdentityFile === activeIdentityFile){
      btnAct.textContent = '当前已激活';
      btnAct.disabled = true;
    }else{
      btnAct.textContent = '激活此身份';
      btnAct.disabled = false;
    }

    const box = document.getElementById('identity-file-list');
    box.innerHTML='';
    for(const f of d.files){
      const el=document.createElement('button');
      el.type='button';
      el.className='identity-file-item'+(f===currentIdentityFile?' active':'');
      el.innerHTML = `<span class="identity-file-name">${esc(f)}</span>${f===activeIdentityFile?'<span class="identity-file-badge">当前</span>':''}`;
      el.onclick = ()=>{
        currentIdentityFile = f;
        loadIdentity();
      };
      box.appendChild(el);
    }
  }catch(e){}
}
async function saveIdentity(){
  const btn=document.getElementById('btn-save-identity');
  const val=document.getElementById('identity-input').value;
  btn.disabled=true; btn.textContent='保存中…';
  try{
    const r=await fetch('/api/identity',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({file:currentIdentityFile,content:val,set_active:false})});
    if(r.ok){
      btn.textContent='已保存'; setTimeout(()=>btn.textContent='保存修改',2000);
      await loadIdentity();
    }
    else alert('保存失败');
  }catch(e){alert('网络错误')}
  finally{btn.disabled=false}
}
async function activateIdentity(){
  try{
    const val=document.getElementById('identity-input').value;
    await fetch('/api/identity',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({file:currentIdentityFile,content:val,set_active:true})});
    await loadIdentity();
  }catch(e){}
}
window.openNewIdentityModal = function(){
  let p = prompt('请输入新身份文件名 (例如: 猫娘.md)');
  if(p && p.trim()) {
    p = p.trim();
    if(!p.includes('.')) p += '.md';
    currentIdentityFile = p;
    document.getElementById('identity-input').value = '';
    saveIdentity();
  }
};

function fmtBytes(n){
  if(n==null) return '—';
  if(n < 1024) return `${n} B`;
  if(n < 1024*1024) return `${(n/1024).toFixed(1)} KB`;
  return `${(n/1024/1024).toFixed(2)} MB`;
}

function fmtUnix(ts){
  if(!ts) return '—';
  return new Date(ts*1000).toLocaleString('zh-CN',{hour12:false});
}

function stopContextAutoRefresh(){
  if(contextAutoRefreshTimer){
    clearInterval(contextAutoRefreshTimer);
    contextAutoRefreshTimer = null;
  }
}

function startContextAutoRefresh(){
  stopContextAutoRefresh();
  contextAutoRefreshTimer = setInterval(()=>{
    if(currentTab==='context-files') loadContextFiles();
  }, 5000);
}

window.toggleContextAutoRefresh = function(enabled){
  contextAutoRefreshEnabled = !!enabled;
  if(contextAutoRefreshEnabled && currentTab==='context-files') startContextAutoRefresh();
  else stopContextAutoRefresh();
};

window.refreshContextFiles = function(){ loadContextFiles(); };

window.copyCurrentContextFile = async function(){
  const text = document.getElementById('context-file-content')?.textContent || '';
  if(!text) return;
  try{
    await navigator.clipboard.writeText(text);
    alert('已复制到剪贴板');
  }catch(e){
    alert('复制失败，请手动复制');
  }
};

async function loadContextFiles(){
  try{
    const query = currentContextFile ? `?file=${encodeURIComponent(currentContextFile)}` : '';
    const r = await fetch('/api/context-files'+query);
    if(!r.ok) throw new Error(await r.text());
    const d = await r.json();
    currentContextFile = d.selected || currentContextFile;

    const list = document.getElementById('context-files-list');
    list.innerHTML = '';
    for(const f of (d.files||[])){
      const item = document.createElement('button');
      item.className = 'context-file-item'+(f.id===d.selected?' active':'');
      const badges = [f.active_in_prompt ? '注入中' : '未注入', f.exists ? '存在' : '缺失', f.is_virtual ? '虚拟' : '文件'];
      item.innerHTML = `<div>${esc(f.label)}</div><div class="context-file-sub">${esc(f.path)} · ${badges.join(' / ')}</div>`;
      item.onclick = ()=>{ currentContextFile = f.id; loadContextFiles(); };
      list.appendChild(item);
    }

    const selectedMeta = (d.files||[]).find(f=>f.id===d.selected);
    document.getElementById('context-selected-name').textContent = selectedMeta ? selectedMeta.label : '未选择文件';
    document.getElementById('context-selected-meta').textContent = selectedMeta
      ? `${selectedMeta.path} · ${selectedMeta.exists?'存在':'缺失'} · ${fmtBytes(selectedMeta.size_bytes)} · 修改时间 ${fmtUnix(selectedMeta.modified_unix)}`
      : '—';
    document.getElementById('context-mode').textContent = d.mode==='aieos' ? 'AIEOS' : 'OpenClaw';
    document.getElementById('context-file-content').textContent = d.content || '';

    const notes = document.getElementById('context-notes');
    notes.innerHTML = (d.notes||[]).map(n=>`<div class="context-note">${esc(n)}</div>`).join('');

    const autoRefresh = document.getElementById('context-auto-refresh');
    if(autoRefresh) autoRefresh.checked = contextAutoRefreshEnabled;
  }catch(e){
    const content = document.getElementById('context-file-content');
    if(content) content.textContent = `加载失败: ${e.message||e}`;
  }
}

async function loadCron(){
  const box=document.getElementById('cron-list');
  try{
    const r=await fetch('/api/cron');
    if(!r.ok) throw new Error();
    const d=await r.json();
    const jobs=d.jobs||[];
    box.innerHTML='';
    if(jobs.length===0){
      box.innerHTML='<div class="empty-state roomy"><span class="material-symbols-outlined">event_busy</span><p>暂无定时任务</p></div>';
      return;
    }
    for(const j of jobs){
      const el=document.createElement('div');
      el.className='expandable-item';
      el.innerHTML=`
        <div class="expandable-header">
          <div class="expandable-icon"><span class="material-symbols-outlined">schedule</span></div>
          <div class="expandable-main">
            <div class="expandable-title">${esc(j.expression)}</div>
            <div class="expandable-subtitle">$ ${esc(j.command||j.prompt||'')}</div>
          </div>
          <div class="cron-status ${j.enabled?'enabled':'disabled'}">${j.enabled?'已启用':'已禁用'}</div>
          <button class="btn btn-text btn-sm" onclick="toggleCron('${j.id}', ${!j.enabled})">${j.enabled?'禁用':'启用'}</button>
          <button class="btn btn-error btn-sm" onclick="deleteCron('${j.id}')">删除</button>
        </div>
      `;
      box.appendChild(el);
    }
  }catch(e){ box.innerHTML='<div class="empty-state">加载失败</div>'; }
}
window.toggleCron = async function(id, enabled){
  await fetch('/api/cron',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:'Toggle',payload:{id,enabled}})});
  loadCron();
};
window.deleteCron = async function(id){
  if(!confirm('确定删除此定时任务吗？')) return;
  await fetch('/api/cron',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:'Delete',payload:{id}})});
  loadCron();
};
window.openCronAddModal = function(){
  const expr = prompt('请输入 Cron 表达式 (例如: */5 * * * *)');
  if(!expr) return;
  const cmd = prompt('请输入 Shell 命令');
  if(!cmd) return;
  fetch('/api/cron',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:'AddShell',payload:{expression:expr,command:cmd}})})
    .then(loadCron);
};

window.serviceCmd = async function(action){
  try{
    const r=await fetch('/api/service',{method:'POST', headers:{'Content-Type':'application/json'},body:JSON.stringify({action})});
    const d=await r.json().catch(()=>({}));
    if(r.ok) alert('执行成功 ('+action+')');
    else alert('执行失败: '+(d.error||'请检查终端日志或权限'));
  }catch(e){alert('网络错误')}
};

function appendMsg(role,text){
  const es=document.getElementById('empty-state');
  if(es) es.remove();
  const box=document.getElementById('messages');
  const div=document.createElement('div');
  div.className='message '+role;
  if(role==='error'){
    div.innerHTML='<span class="material-symbols-outlined">error</span>'+esc(text);
  } else {
    div.textContent=text;
  }
  box.appendChild(div);
  box.scrollTop=box.scrollHeight;
  return div;
}
function showLoading(){
  const es=document.getElementById('empty-state');
  if(es) es.remove();
  const box=document.getElementById('messages');
  const div=document.createElement('div');
  div.className='message assistant';div.id='loading-msg';
  div.innerHTML='<div class="loading-dots"><span></span><span></span><span></span></div>';
  box.appendChild(div);box.scrollTop=box.scrollHeight;
}
function hideLoading(){
  const el=document.getElementById('loading-msg');
  if(el) el.remove();
}
window.sendMessage = async function(){
  const inp=document.getElementById('msg-input');
  const text=inp.value.trim();
  if(!text) return;
  inp.value='';inp.style.height='auto';
  const btn=document.getElementById('send-btn');
  btn.disabled=true;
  appendMsg('user',text);
  showLoading();
  try{
    const r=await fetch('/api/chat',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({message:text})});
    hideLoading();
    const d=await r.json();
    if(!r.ok) appendMsg('error',d.error||'请求失败');
    else appendMsg('assistant',d.response);
  }catch(e){
    hideLoading();
    appendMsg('error','网络错误，请检查连接');
  }finally{
    btn.disabled=false;
    inp.focus();
  }
};

loadStatus();
