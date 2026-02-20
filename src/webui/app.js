const pageTitles = {
  dashboard:'仪表盘', chat:'对话', history:'聊天记录', channels:'消息平台',
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
  if(name==='chat') loadWebuiChatHistory();
  if(name==='history') loadConversations();
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
  if(currentTab==='chat') loadWebuiChatHistory();
  if(currentTab==='history') loadConversations();
  if(currentTab==='context-files') loadContextFiles();
}

// ── Conversation history ─────────────────────────────────────────────────────
let _conversationKind = 'private';
const WEBUI_GLOBAL_SESSION_ID = 'webui:global';
let _selectedConversationSession = '';

function fmtLocalTimeFromUnix(ts){
  if(!ts) return '';
  try{ return new Date(ts*1000).toLocaleString(); }catch(e){ return String(ts); }
}

function formatConversationTarget(kind, target){
  const t = (target||'').trim();
  if(!t) return '';
  if(t==='global' || kind==='global') return '全局会话';

  if(kind==='group' && t.startsWith('group:')) return '群聊 · '+t.slice('group:'.length);
  if(kind==='private' && (t.startsWith('private:')||t.startsWith('user:'))){
    const v = t.replace(/^private:/,'').replace(/^user:/,'');
    return '私聊 · '+v;
  }
  return t;
}

async function loadConversationSettings(){
  try{
    const r = await fetch('/api/config');
    if(!r.ok) return;
    const d = await r.json().catch(()=>({}));
    const limit = d?.agent?.max_history_messages;
    const inp = document.getElementById('history-context-limit');
    if(inp && typeof limit === 'number') inp.value = String(limit);

    const isolate = d?.agent?.isolate_channel_conversations;
    const isolateToggle = document.getElementById('history-isolation-toggle');
    if(isolateToggle && typeof isolate === 'boolean') isolateToggle.checked = isolate;

    const hint = document.getElementById('history-hint');
    if(hint && typeof limit === 'number'){
      const isolateTxt = isolate===false ? '关闭（频道将共用全局上下文）' : '开启（私聊/群聊按会话隔离）';
      hint.textContent = `当前配置：每次对话将携带最近 ${limit} 条历史消息；频道隔离：${isolateTxt}（保存后通常需要重启 daemon/channels 才会生效）。`;
    }
  }catch(e){}
}

window.setConversationKind = function(kind){
  _conversationKind = kind || 'private';
  ['private','group','global','other'].forEach(k=>{
    const btn = document.getElementById('history-kind-'+k);
    if(btn) btn.classList.toggle('active', _conversationKind===k);
  });
  _selectedConversationSession = '';
  const msgs = document.getElementById('history-messages');
  if(msgs){
    msgs.innerHTML = '<div class="empty-state" id="history-empty"><span class="material-symbols-outlined">forum</span><p>选择左侧会话查看消息</p></div>';
  }
  loadConversations();
};

window.loadConversations = async function(){
  await loadConversationSettings();
  const box = document.getElementById('history-sessions');
  if(!box) return;
  box.innerHTML = '<div class="empty-state compact"><span class="material-symbols-outlined">hourglass_empty</span><p>正在加载会话…</p></div>';

  try{
    let url = '/api/conversations?limit=200';
    if(_conversationKind==='group' || _conversationKind==='private'){
      url += `&channel=onebot_v11&kind=${encodeURIComponent(_conversationKind)}`;
    } else {
      url += `&kind=${encodeURIComponent(_conversationKind||'other')}`;
    }
    const r = await fetch(url);
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'加载失败');
    const sessions = Array.isArray(d.sessions) ? d.sessions : [];

    if(!sessions.length){
      box.innerHTML = '<div class="empty-state compact"><span class="material-symbols-outlined">inbox</span><p>暂无会话</p></div>';
      return;
    }

    box.innerHTML = '';
    for(const s of sessions){
      const item = document.createElement('div');
      item.className = 'history-session-item'+(s.session_id===_selectedConversationSession?' active':'');
      const title = formatConversationTarget(s.kind, s.target) || s.session_id;
      const sub = `${s.channel||''} · ${s.last_timestamp||''}`.trim();
      item.innerHTML = `<div class="history-session-title">${esc(title)}</div><div class="history-session-sub">${esc(sub)}</div>`;
      item.onclick = ()=> selectConversation(s.session_id);
      box.appendChild(item);
    }
  }catch(e){
    box.innerHTML = '<div class="empty-state compact"><span class="material-symbols-outlined">error</span><p>'+esc(e.message||'加载失败')+'</p></div>';
  }
};

async function selectConversation(sessionId){
  _selectedConversationSession = sessionId;
  // refresh highlight
  document.querySelectorAll('#history-sessions .history-session-item').forEach(el=>el.classList.remove('active'));
  // reload sessions list quickly to set active state (cheap DOM update would be better but simple)
  loadConversations();
  await loadConversationMessages(sessionId);
}

async function loadConversationMessages(sessionId){
  const box = document.getElementById('history-messages');
  if(!box) return;
  box.innerHTML = '<div class="empty-state"><span class="material-symbols-outlined">hourglass_empty</span><p>正在加载消息…</p></div>';

  try{
    const rawLimit = Number(document.getElementById('history-context-limit')?.value || 0);
    const limit = Number.isFinite(rawLimit) && rawLimit > 0 ? Math.floor(rawLimit) : 2000;
    const url = `/api/conversations/messages?session_id=${encodeURIComponent(sessionId)}&limit=${Math.min(Math.max(limit,1),5000)}`;
    const r = await fetch(url);
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'加载失败');
    const msgs = Array.isArray(d.messages) ? d.messages : [];
    if(!msgs.length){
      box.innerHTML = '<div class="empty-state"><span class="material-symbols-outlined">inbox</span><p>该会话暂无消息</p></div>';
      return;
    }

    box.innerHTML = '';
    for(const m of msgs){
      const role = (m.role||'user').toLowerCase();
      const div = document.createElement('div');
      div.className = 'message ' + (role==='assistant'?'assistant':'user');
      let meta = '';
      if(role==='assistant'){
        meta = '机器人';
      } else {
        const name = (m.sender_name||'').trim();
        const id = (m.sender_id||'').trim();
        if(name && id) meta = `${name}(${id})`;
        else if(name) meta = name;
        else if(id) meta = id;
        else meta = '用户';
      }
      const ts = fmtLocalTimeFromUnix(m.timestamp);
      if(ts) meta += ' · ' + ts;
      div.innerHTML = `<div class="msg-meta">${esc(meta)}</div><div class="msg-text">${esc(m.text||'')}</div>`;
      box.appendChild(div);
    }
    box.scrollTop = box.scrollHeight;
  }catch(e){
    box.innerHTML = '<div class="empty-state"><span class="material-symbols-outlined">error</span><p>'+esc(e.message||'加载失败')+'</p></div>';
  }
}

async function clearConversationRecords(payload, confirmMessage){
  if(!confirm(confirmMessage)) return;
  try{
    const r = await fetch('/api/conversations/clear',{
      method:'POST',
      headers:{'Content-Type':'application/json'},
      body: JSON.stringify(payload||{})
    });
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'清空失败');

    const removed = Number(d.removed||0);
    const skipped = Number(d.skipped||0);
    if(skipped>0){
      alert(`已处理，删除 ${removed} 条，跳过 ${skipped} 条（当前内存后端可能不支持删除）。`);
    }else{
      alert(`已删除 ${removed} 条聊天记录。`);
    }
  }catch(e){
    alert('清空失败: '+(e.message||e));
    return;
  }

  _selectedConversationSession = '';
  const msgs = document.getElementById('history-messages');
  if(msgs){
    msgs.innerHTML = '<div class="empty-state" id="history-empty"><span class="material-symbols-outlined">forum</span><p>选择左侧会话查看消息</p></div>';
  }
  loadConversations();
}

window.clearSelectedConversation = async function(){
  if(!_selectedConversationSession){
    alert('请先选择要清空的会话');
    return;
  }
  await clearConversationRecords(
    { session_id: _selectedConversationSession },
    `确定清空会话 ${_selectedConversationSession} 的聊天记录吗？此操作不可恢复。`
  );
};

window.clearAllConversations = async function(){
  await clearConversationRecords(
    { all: true },
    '确定清空全部聊天记录吗？此操作不可恢复。'
  );
};



window.saveConversationContextLimit = async function(){
  const inp = document.getElementById('history-context-limit');
  if(!inp) return;
  const v = Number(inp.value||0);
  if(!Number.isFinite(v) || v<0){
    alert('请输入合法的数字');
    return;
  }
  try{
    const isolateToggle = document.getElementById('history-isolation-toggle');
    const payload = {
      max_history_messages: Math.floor(v),
      isolate_channel_conversations: isolateToggle ? !!isolateToggle.checked : undefined,
    };
    const r = await fetch('/api/config',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:'Agent',payload})});
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'保存失败');
    alert('已保存（通常需要重启 daemon/channels 才会生效）');
    loadConversationSettings();
  }catch(e){
    alert('保存失败: '+(e.message||e));
  }
};


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
    const names={cli:'CLI',telegram:'Telegram',discord:'Discord',slack:'Slack',webhook:'Webhook',whatsapp:'WhatsApp',qq:'QQ Official',onebot_v11:'NapCat/OneBot v11'};
    const icons={cli:'terminal',telegram:'send',discord:'headset_mic',slack:'tag',webhook:'webhook',whatsapp:'chat',qq:'smart_toy',onebot_v11:'hub'};
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

  const askRestartNow = async function(){
    const yes = confirm('检测到部分配置需要重启 ZeroClaw 才能生效。\n\n是否现在尝试在前端直接重启服务？\n\n（若未安装服务，可先到「设置 → 系统守护进程」安装后再试）');
    if(!yes){
      alert('配置已保存。你可以稍后在「设置 → 系统守护进程」中点击“重启服务”。');
      return;
    }
    await window.serviceCmd('restart');
  };

  try{
    const r=await fetch('/api/config/raw',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(_rawConfig)});
    if(r.ok){
        const resp = await r.json();
        if(resp.requires_restart) await askRestartNow();
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
  {key:'onebot_v11', name:'NapCat / OneBot v11', icon:'hub', desc:'NapCat 作为 OneBot v11 接入', schema:{api_url:'str',access_token:'str',listen_host:'str',listen_port:'num',allowed_users:'arr',allowed_groups:'arr',require_at_in_group:'bool',admin_users:'arr',admin_only_tools:'arr',command_external_network_access:'str',non_admin_context_file:'str'}},
  {key:'webhook',  name:'Webhook', icon:'webhook', desc:'HTTP 回调接口', schema:{port:'num',secret:'str'}},
  {key:'whatsapp', name:'WhatsApp', icon:'chat', desc:'Meta Business API', schema:{access_token:'str',phone_number_id:'str',verify_token:'str',app_secret:'str',allowed_numbers:'arr'}},
  {key:'lark',     name:'飞书 / Lark', icon:'apartment', desc:'飞书开放平台', schema:{app_id:'str',app_secret:'str',encrypt_key:'str',verification_token:'str',allowed_users:'arr',use_feishu:'bool',receive_mode:'str',port:'num'}},
  {key:'dingtalk', name:'钉钉 DingTalk', icon:'campaign', desc:'钉钉 Stream 模式', schema:{client_id:'str',client_secret:'str',allowed_users:'arr'}},
  {key:'cli',      name:'CLI 终端', icon:'terminal', desc:'启用命令行交互通道', schema:{}},
];

const CH_DICT = {bot_token:'机器人 Token', allowed_users:'授权用户ID (逗号分隔)', guild_id:'服务器 ID (Guild)', listen_to_bots:'监听其他机器人', mention_only:'仅响应@提及', app_token:'App Token', channel_id:'频道 ID', app_id:'应用 ID', app_secret:'应用密钥', port:'监听端口', secret:'密钥 (可选)', access_token:'访问令牌', phone_number_id:'电话号码ID', verify_token:'验证令牌', allowed_numbers:'授权电话号码', encrypt_key:'加密密钥', verification_token:'事件订阅Token', use_feishu:'使用飞书(而非Lark)', receive_mode:'接收模式(websocket/webhook)', client_id:'Client ID', client_secret:'Client Secret', api_url:'OneBot API 地址', listen_host:'回调监听地址', listen_port:'回调监听端口', allowed_groups:'授权群号 (逗号分隔，可空=全部)', require_at_in_group:'群聊仅响应@机器人', admin_users:'管理员QQ (逗号分隔)', admin_only_tools:'仅管理员可用工具 (逗号分隔)', command_external_network_access:'命令外网访问策略', non_admin_context_file:'非管理员上下文文件路径'};


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
      if(c.key==='onebot_v11' && k==='command_external_network_access'){
        const mode = (val || 'off').toString();
        fg.innerHTML=`<label>${esc(label)}</label>
          <select class="form-input" data-path="channels_config.${c.key}.${k}">
            <option value="off" ${mode==='off'?'selected':''}>关闭（禁止命令外网访问）</option>
            <option value="on" ${mode==='on'?'selected':''}>开启（允许所有 OneBot 会话）</option>
            <option value="admin_only" ${mode==='admin_only'?'selected':''}>仅管理员</option>
          </select>`;
      } else if(type==='bool'){
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
         if(key === 'onebot_v11') {
             _rawConfig.channels_config.onebot_v11.api_url = 'http://127.0.0.1:3000';
             _rawConfig.channels_config.onebot_v11.listen_host = '0.0.0.0';
             _rawConfig.channels_config.onebot_v11.listen_port = 8096;
             _rawConfig.channels_config.onebot_v11.allowed_users = [];
             _rawConfig.channels_config.onebot_v11.allowed_groups = [];
             _rawConfig.channels_config.onebot_v11.require_at_in_group = true;
             _rawConfig.channels_config.onebot_v11.admin_users = [];
             _rawConfig.channels_config.onebot_v11.admin_only_tools = [];
             _rawConfig.channels_config.onebot_v11.command_external_network_access = 'off';
             _rawConfig.channels_config.onebot_v11.non_admin_context_file = 'NON_ADMIN.md';
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
  const isRestart = action === 'restart';
  try{
    const r=await fetch('/api/service',{method:'POST', headers:{'Content-Type':'application/json'},body:JSON.stringify({action})});
    const d=await r.json().catch(()=>({}));
    if(r.ok){
      if(isRestart) alert('已触发重启。WebUI 可能会短暂断开，请等待 3-10 秒后刷新页面确认。');
      else alert('执行成功 ('+action+')');
    }
    else alert('执行失败: '+(d.error||'请检查终端日志或权限'));
  }catch(e){
    if(isRestart) alert('重启请求已发出，连接可能因进程重启而中断。请等待几秒后刷新页面。');
    else alert('网络错误');
  }
};

async function loadWebuiChatHistory(){
  const box = document.getElementById('messages');
  if(!box) return;

  try{
    const rawLimit = Number(document.getElementById('history-context-limit')?.value || 0);
    const limit = Number.isFinite(rawLimit) && rawLimit > 0 ? Math.floor(rawLimit) : 2000;
    const url = `/api/conversations/messages?session_id=${encodeURIComponent(WEBUI_GLOBAL_SESSION_ID)}&limit=${Math.min(Math.max(limit,1),5000)}`;
    const r = await fetch(url);
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'加载失败');
    const msgs = Array.isArray(d.messages) ? d.messages : [];

    box.innerHTML = '';
    if(!msgs.length){
      box.innerHTML = '<div class="empty-state" id="empty-state"><span class="material-symbols-outlined">forum</span><p>发送消息开始对话</p></div>';
      return;
    }

    for(const m of msgs){
      const role = (m.role||'user').toLowerCase() === 'assistant' ? 'assistant' : 'user';
      const div = document.createElement('div');
      div.className = 'message ' + role;
      div.textContent = m.text || '';
      box.appendChild(div);
    }
    box.scrollTop = box.scrollHeight;
  }catch(e){
    box.innerHTML = '<div class="empty-state" id="empty-state"><span class="material-symbols-outlined">error</span><p>'+esc(e.message||'加载历史失败')+'</p></div>';
  }
}


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

// ── First-run Onboarding Wizard (WebUI) ─────────────────────────────────────

let _onboardOpen = false;
let _onboardStep = 0;
let _onboardMode = 'full'; // 'full' | 'files'
let _onboardStatus = null;

function onboardOverlayEl(){ return document.getElementById('onboard-overlay'); }
function onboardIsActive(){ return onboardOverlayEl()?.classList.contains('active'); }

function setOnboardOverlay(active){
  const el = onboardOverlayEl();
  if(!el) return;
  el.classList.toggle('active', !!active);
  el.setAttribute('aria-hidden', active ? 'false' : 'true');
  document.body.style.overflow = active ? 'hidden' : '';
}

async function fetchOnboardStatus(){
  try{
    const r = await fetch('/api/onboard/status');
    if(!r.ok) return null;
    return await r.json();
  }catch(e){
    return null;
  }
}

function renderOnboardChecklist(){
  const box = document.getElementById('ob-workspace-checklist');
  if(!box) return;
  const ws = _onboardStatus?.workspace || {};
  const presentFiles = new Set(ws.present_files||[]);
  const missingFiles = new Set(ws.missing_files||[]);
  const presentDirs = new Set(ws.present_subdirs||[]);
  const missingDirs = new Set(ws.missing_subdirs||[]);

  function item(name, ok){
    return `\
      <div class="onboard-check-item">\
        <div class="onboard-check-left">\
          <span class="material-symbols-outlined">${ok?'check_circle':'error'}</span>\
          <span class="onboard-check-name">${esc(name)}</span>\
        </div>\
        <span class="onboard-check-badge ${ok?'ok':'miss'}">${ok?'已存在':'缺失'}</span>\
      </div>`;
  }

  let html = '';
  html += '<div class="section-title"><span class="material-symbols-outlined">folder</span>目录</div>';
  const dirs = [...presentDirs, ...missingDirs].sort();
  for(const d of dirs){
    html += item(d+'/', presentDirs.has(d));
  }

  html += '<div class="section-title" style="margin-top:10px"><span class="material-symbols-outlined">description</span>上下文文件</div>';
  const files = [...presentFiles, ...missingFiles].sort();
  for(const f of files){
    html += item(f, presentFiles.has(f));
  }

  box.innerHTML = html || '—';

  // default scaffold checkbox
  const sc = document.getElementById('ob-scaffold-files');
  const hasWorkspaceReport =
    Array.isArray(ws.present_files) || Array.isArray(ws.missing_files) ||
    Array.isArray(ws.present_subdirs) || Array.isArray(ws.missing_subdirs);
  if(sc && (!hasWorkspaceReport || missingFiles.size || missingDirs.size)){
    sc.checked = true;
  }
}

function fillOnboardFormsFromConfig(){
  if(!_rawConfig) return;
  const providerSel = document.getElementById('ob-provider');
  const modelInp = document.getElementById('ob-model');
  const apiKeyInp = document.getElementById('ob-api-key');
  const apiUrlInp = document.getElementById('ob-api-url');

  const rawProvider = (_rawConfig.default_provider||'').trim();
  const isCustom = rawProvider.startsWith('custom:');
  const providerId = isCustom ? 'compatible' : (stripCustom(rawProvider).toLowerCase()||'openrouter');
  if(providerSel){
    // If provider not in options, keep existing as compatible.
    const opt = providerSel.querySelector(`option[value="${providerId}"]`);
    providerSel.value = opt ? providerId : (isCustom?'compatible':'openrouter');
  }
  if(apiUrlInp){
    apiUrlInp.value = isCustom ? stripCustom(rawProvider) : (_rawConfig.api_url||'');
  }
  if(modelInp) modelInp.value = _rawConfig.default_model || '';
  if(apiKeyInp){
    apiKeyInp.value = '';
    apiKeyInp.placeholder = _rawConfig.api_key ? '•••••••• (已配置)' : '未配置';
  }

  // Channels
  const ch = _rawConfig.channels_config || {};

  const onebotOn = !!ch.onebot_v11;
  const onebotCk = document.getElementById('ob-ch-onebot');
  if(onebotCk) onebotCk.checked = onebotOn;
  if(onebotOn){
    document.getElementById('ob-onebot-api-url').value = ch.onebot_v11.api_url || '';
    document.getElementById('ob-onebot-token').value = '';
    document.getElementById('ob-onebot-token').placeholder = ch.onebot_v11.access_token ? '•••••••• (已配置)' : '不输入则为空';
    document.getElementById('ob-onebot-listen-host').value = ch.onebot_v11.listen_host || '';
    document.getElementById('ob-onebot-listen-port').value = ch.onebot_v11.listen_port ?? 8096;
    document.getElementById('ob-onebot-allowed-users').value = arr2str(ch.onebot_v11.allowed_users||[]);
    document.getElementById('ob-onebot-allowed-groups').value = arr2str(ch.onebot_v11.allowed_groups||[]);
    document.getElementById('ob-onebot-require-at').checked = !!ch.onebot_v11.require_at_in_group;
    document.getElementById('ob-onebot-net-access').value = ch.onebot_v11.command_external_network_access || 'off';
  }

  const tgOn = !!ch.telegram;
  const tgCk = document.getElementById('ob-ch-telegram');
  if(tgCk) tgCk.checked = tgOn;
  if(tgOn){
    document.getElementById('ob-telegram-token').value = '';
    document.getElementById('ob-telegram-token').placeholder = ch.telegram.bot_token ? '•••••••• (已配置)' : '123:ABC...';
    document.getElementById('ob-telegram-allowed').value = arr2str(ch.telegram.allowed_users||[]);
  }

  const dsOn = !!ch.discord;
  const dsCk = document.getElementById('ob-ch-discord');
  if(dsCk) dsCk.checked = dsOn;
  if(dsOn){
    document.getElementById('ob-discord-token').value = '';
    document.getElementById('ob-discord-token').placeholder = ch.discord.bot_token ? '•••••••• (已配置)' : '';
    document.getElementById('ob-discord-guild').value = ch.discord.guild_id || '';
  }

  const whOn = !!ch.webhook;
  const whCk = document.getElementById('ob-ch-webhook');
  if(whCk) whCk.checked = whOn;
  if(whOn){
    document.getElementById('ob-webhook-port').value = ch.webhook.port ?? 8080;
    document.getElementById('ob-webhook-secret').value = '';
    document.getElementById('ob-webhook-secret').placeholder = ch.webhook.secret ? '•••••••• (已配置)' : '';
  }

  // Tunnel
  const tSel = document.getElementById('ob-tunnel-provider');
  if(tSel) tSel.value = (_rawConfig.tunnel?.provider || 'none');
  document.getElementById('ob-tunnel-cf-token').value = '';
  document.getElementById('ob-tunnel-ngrok-token').value = '';
  document.getElementById('ob-tunnel-custom-cmd').value = _rawConfig.tunnel?.custom?.start_command || '';
  document.getElementById('ob-tunnel-custom-health').value = _rawConfig.tunnel?.custom?.health_url || '';
  document.getElementById('ob-tunnel-custom-pattern').value = _rawConfig.tunnel?.custom?.url_pattern || '';
  document.getElementById('ob-tunnel-ts-funnel').checked = !!_rawConfig.tunnel?.tailscale?.funnel;
  document.getElementById('ob-tunnel-ts-host').value = _rawConfig.tunnel?.tailscale?.hostname || '';
  document.getElementById('ob-tunnel-ngrok-domain').value = _rawConfig.tunnel?.ngrok?.domain || '';

  // Tool/Security
  document.getElementById('ob-autonomy-level').value = (_rawConfig.autonomy?.level || 'supervised');
  document.getElementById('ob-secrets-encrypt').checked = _rawConfig.secrets?.encrypt !== false;
  document.getElementById('ob-composio-enabled').checked = !!_rawConfig.composio?.enabled;
  document.getElementById('ob-composio-api-key').value = '';
  document.getElementById('ob-composio-api-key').placeholder = _rawConfig.composio?.api_key ? '•••••••• (已配置)' : '';
  document.getElementById('ob-composio-entity').value = _rawConfig.composio?.entity_id || 'default';

  // Hardware
  document.getElementById('ob-hw-enabled').checked = !!_rawConfig.hardware?.enabled;
  document.getElementById('ob-hw-transport').value = (_rawConfig.hardware?.transport || 'None');
  document.getElementById('ob-hw-serial-port').value = _rawConfig.hardware?.serial_port || '';
  document.getElementById('ob-hw-baud').value = _rawConfig.hardware?.baud_rate ?? 115200;
  document.getElementById('ob-hw-probe-target').value = _rawConfig.hardware?.probe_target || '';
  document.getElementById('ob-hw-datasheets').checked = !!_rawConfig.hardware?.workspace_datasheets;

  // Memory
  document.getElementById('ob-mem-backend').value = (_rawConfig.memory?.backend || 'sqlite');
  document.getElementById('ob-mem-autosave').checked = _rawConfig.memory?.auto_save !== false;
  document.getElementById('ob-mem-retention').value = _rawConfig.memory?.conversation_retention_days ?? 30;
  document.getElementById('ob-agent-max-history').value = _rawConfig.agent?.max_history_messages ?? 100;

  // Project context defaults (best-effort)
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  const u = document.getElementById('ob-user-name');
  const a = document.getElementById('ob-agent-name');
  const t = document.getElementById('ob-timezone');
  if(u && !u.value) u.value = 'User';
  if(a && !a.value) a.value = 'ZeroClaw';
  if(t && !t.value) t.value = tz;
  const stylePreset = document.getElementById('ob-style-preset');
  if(stylePreset && !stylePreset.value) stylePreset.value = 'friendly';
  const langSel = document.getElementById('ob-comm-language');
  const langCustom = document.getElementById('ob-lang-custom');
  if(langSel && !langSel.dataset.inited){
    const inferred = inferOnboardLanguagePreset();
    if(inferred === 'custom' && langCustom && !langCustom.value) langCustom.value = navigator.language || 'English';
    langSel.value = inferred;
    langSel.dataset.inited = '1';
  }
}

function renderOnboardStep(){
  // Stepper active
  document.querySelectorAll('.onboard-step-btn').forEach(btn=>{
    const s = Number(btn.getAttribute('data-step')||0);
    btn.classList.toggle('active', s===_onboardStep);
  });

  // Content
  document.querySelectorAll('.onboard-step').forEach(el=>{
    const s = Number(el.getAttribute('data-step')||0);
    el.classList.toggle('active', s===_onboardStep);
  });

  // Buttons
  const btnPrev = document.getElementById('ob-btn-prev');
  const btnNext = document.getElementById('ob-btn-next');
  if(btnPrev) btnPrev.disabled = _onboardStep<=0 || _onboardMode==='files';
  if(btnNext) btnNext.textContent = (_onboardStep>=8) ? '完成并应用' : '下一步';

  // Hint
  const hint = document.getElementById('ob-footer-hint');
  if(hint){
    if(_onboardStep===2){
      hint.textContent = '提示：OneBot v11 的「授权 QQ」留空会拒绝全部用户；填写 * 允许全部。';
    } else if(_onboardStep===8){
      hint.textContent = '最后一步：建议先生成工作区文件，再保存配置。';
    } else {
      hint.textContent = '';
    }
  }

  // Mobile: if in files mode, hide stepper
  const stepper = document.querySelector('.onboard-stepper');
  if(stepper){
    stepper.style.display = (_onboardMode==='files') ? 'none' : '';
  }

  // Ensure UI sections are correctly shown
  onboardProviderChanged();
  onboardTunnelChanged();
  onboardComposioChanged();
  onboardHardwareChanged();
  onboardStylePresetChanged();
  onboardLanguageChanged();
  ['onebot','telegram','discord','webhook'].forEach(k=>onboardToggleChannel(k, true));
}

async function loadRawConfigForOnboard(){
  const r = await fetch('/api/config/raw');
  if(!r.ok) throw new Error('无法加载配置');
  _rawConfig = await r.json();
}

window.openOnboardWizard = async function(force, mode){
  _onboardMode = mode || 'full';
  _onboardOpen = true;
  _onboardStep = 0;
  setOnboardOverlay(true);

  // Clear finish output
  const fin = document.getElementById('ob-finish-result');
  if(fin) fin.innerHTML = '';

  try{
    await loadRawConfigForOnboard();
  }catch(e){
    // keep going; wizard can still scaffold files
  }

  _onboardStatus = await fetchOnboardStatus();
  renderOnboardChecklist();
  fillOnboardFormsFromConfig();
  renderOnboardStep();

  // Special modes
  if(_onboardMode==='files'){
    _onboardStep = 8;
    renderOnboardStep();
  }
};

window.closeOnboardWizard = function(){
  _onboardOpen = false;
  setOnboardOverlay(false);
};

window.onboardSkip = async function(){
  try{ await fetch('/api/onboard/mark-done',{method:'POST'}); }catch(e){}
  window.closeOnboardWizard();
};

window.gotoOnboardStep = function(step){
  if(_onboardMode==='files') return;
  _onboardStep = Math.max(0, Math.min(8, Number(step||0)));
  renderOnboardStep();
};

window.onboardPrev = function(){
  if(_onboardMode==='files') return;
  _onboardStep = Math.max(0, _onboardStep-1);
  renderOnboardStep();
};

window.onboardNext = async function(){
  const btnNext = document.getElementById('ob-btn-next');
  if(btnNext) btnNext.disabled = true;
  try{
    if(_onboardStep < 8 && _onboardMode!=='files'){
      _onboardStep += 1;
      renderOnboardStep();
      return;
    }

    const res = await onboardFinishApply();
    if(res?.requires_restart){
      const yes = confirm('配置已保存，但部分修改需要重启 ZeroClaw 才会完全生效。\n\n是否现在尝试在前端直接重启服务？');
      if(yes){
        await window.serviceCmd('restart');
      } else {
        alert('配置已保存。你可以稍后在「设置 → 系统守护进程」中点击“重启服务”。');
      }
    }
    // Auto close after successful apply
    window.closeOnboardWizard();
  } catch(e) {
    const msg = (e && e.message) ? e.message : String(e);
    const fin = document.getElementById('ob-finish-result');
    if(fin) fin.innerHTML += `<div class="context-note">应用失败：${esc(msg)}</div>`;
    alert('应用失败：' + msg);
  } finally {
    if(btnNext) btnNext.disabled = false;
  }
};

window.onboardProviderChanged = function(){
  const provider = document.getElementById('ob-provider')?.value || 'openrouter';
  const wrap = document.getElementById('ob-api-url-wrap');
  if(wrap) wrap.style.display = (provider==='compatible') ? '' : 'none';
};

window.onboardTunnelChanged = function(){
  const provider = document.getElementById('ob-tunnel-provider')?.value || 'none';
  const boxes = {
    cloudflare: document.getElementById('ob-tunnel-cloudflare'),
    tailscale: document.getElementById('ob-tunnel-tailscale'),
    ngrok: document.getElementById('ob-tunnel-ngrok'),
    custom: document.getElementById('ob-tunnel-custom')
  };
  Object.entries(boxes).forEach(([k,el])=>{ if(el) el.style.display = (k===provider)?'':'none'; });
};

window.onboardComposioChanged = function(){
  const on = !!document.getElementById('ob-composio-enabled')?.checked;
  const box = document.getElementById('ob-composio-box');
  if(box) box.style.display = on ? '' : 'none';
};

window.onboardHardwareChanged = function(){
  const enabled = !!document.getElementById('ob-hw-enabled')?.checked;
  const transport = document.getElementById('ob-hw-transport')?.value || 'None';
  const serial = document.getElementById('ob-hw-serial');
  const probe = document.getElementById('ob-hw-probe');
  if(serial) serial.style.display = (enabled && transport==='Serial') ? '' : 'none';
  if(probe) probe.style.display = (enabled && transport==='Probe') ? '' : 'none';
};

window.onboardStylePresetChanged = function(){
  const v = document.getElementById('ob-style-preset')?.value || 'friendly';
  const wrap = document.getElementById('ob-style-custom-wrap');
  if(wrap) wrap.style.display = (v==='custom') ? '' : 'none';
};

window.onboardLanguageChanged = function(){
  const v = document.getElementById('ob-comm-language')?.value || 'English';
  const wrap = document.getElementById('ob-lang-custom-wrap');
  if(wrap) wrap.style.display = (v==='custom') ? '' : 'none';
};

window.onboardToggleChannel = function(key, silent){
  const ck = document.getElementById('ob-ch-'+key);
  const body = document.getElementById('ob-ch-'+key+'-body');
  if(!ck || !body) return;
  body.style.display = ck.checked ? '' : 'none';

  if(silent) return;
  // defaults
  if(key==='onebot' && ck.checked){
    if(!document.getElementById('ob-onebot-api-url').value) document.getElementById('ob-onebot-api-url').value = 'http://127.0.0.1:3000';
    if(!document.getElementById('ob-onebot-listen-host').value) document.getElementById('ob-onebot-listen-host').value = '0.0.0.0';
    if(!document.getElementById('ob-onebot-listen-port').value) document.getElementById('ob-onebot-listen-port').value = '8096';
    if(!document.getElementById('ob-onebot-allowed-users').value.trim()) document.getElementById('ob-onebot-allowed-users').value = '*';
    if(document.getElementById('ob-onebot-require-at').checked === false) document.getElementById('ob-onebot-require-at').checked = true;
  }
  if(key==='webhook' && ck.checked){
    if(!document.getElementById('ob-webhook-port').value) document.getElementById('ob-webhook-port').value = '8080';
  }
};

window.onboardDiscoverModels = async function(){
  const btn = document.getElementById('ob-discover-btn');
  const status = document.getElementById('ob-discover-status');
  const sel = document.getElementById('ob-discover-select');
  if(btn){ btn.disabled = true; btn.textContent = '拉取中…'; }
  if(status) status.textContent = '';
  if(sel) sel.innerHTML = '<option value="">选择已发现模型…</option>';
  try{
    const providerId = document.getElementById('ob-provider')?.value || 'openrouter';
    const apiUrl = (document.getElementById('ob-api-url')?.value || '').trim();
    const apiKey = (document.getElementById('ob-api-key')?.value || '').trim();
    const provider = (providerId==='compatible') ? `custom:${apiUrl}` : providerId;
    const payload = { provider };
    if(apiUrl) payload.api_url = apiUrl;
    if(apiKey) payload.api_key = apiKey;
    const r = await fetch('/api/models/discover',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'自动拉取失败');
    const models = Array.isArray(d.models) ? d.models : [];
    if(sel){
      sel.innerHTML = '<option value="">选择已发现模型…</option>' + models.map(m=>`<option value="${esc(m)}">${esc(m)}</option>`).join('');
    }
    if(status) status.textContent = models.length ? `已从 ${d.endpoint||'接口'} 拉取 ${models.length} 个模型` : '接口可达，但未返回模型列表';
  }catch(e){
    if(status) status.textContent = '拉取失败：' + (e.message||e);
  }finally{
    if(btn){ btn.disabled = false; btn.textContent = '从接口自动拉取模型'; }
  }
};

window.onboardApplyDiscoveredModel = function(v){
  if(!v) return;
  const inp = document.getElementById('ob-model');
  if(inp) inp.value = v;
};

function onboardStyleFromPreset(){
  const p = document.getElementById('ob-style-preset')?.value || 'friendly';
  if(p==='direct') return 'Be direct and concise. Skip pleasantries. Get to the point.';
  if(p==='friendly') return 'Be friendly, human, and conversational. Show warmth and empathy while staying efficient. Use natural contractions.';
  if(p==='professional') return 'Be professional and polished. Stay calm, structured, and respectful. Use occasional tone-setting emojis only when appropriate.';
  if(p==='playful') return 'Be expressive and playful when appropriate. Use relevant emojis naturally (0-2 max), and keep serious topics emoji-light.';
  if(p==='technical') return 'Be technical and detailed. Thorough explanations, code-first.';
  if(p==='balanced') return 'Adapt to the situation. Default to warm and clear communication; be concise when needed, thorough when it matters.';
  return (document.getElementById('ob-style-custom')?.value || '').trim();
}

function inferOnboardLanguagePreset(){
  const lang = String(navigator.language || '').toLowerCase();
  if(lang.startsWith('zh-cn') || lang.startsWith('zh-sg')) return '中文（简体）';
  if(lang.startsWith('zh-tw') || lang.startsWith('zh-hk') || lang.startsWith('zh-mo')) return '中文（繁體）';
  if(lang.startsWith('ja')) return '日本語';
  if(lang.startsWith('ko')) return '한국어';
  if(lang.startsWith('es')) return 'Español';
  if(lang.startsWith('fr')) return 'Français';
  if(lang.startsWith('de')) return 'Deutsch';
  if(lang.startsWith('en')) return 'English';
  return 'custom';
}

function onboardLanguageFromPreset(){
  const v = (document.getElementById('ob-comm-language')?.value || '').trim();
  if(v && v !== 'custom') return v;
  const custom = (document.getElementById('ob-lang-custom')?.value || '').trim();
  if(custom) return custom;
  const inferred = inferOnboardLanguagePreset();
  if(inferred && inferred !== 'custom') return inferred;
  return 'English';
}

async function onboardFinishApply(){
  const fin = document.getElementById('ob-finish-result');
  if(fin) fin.innerHTML = '';

  let requiresRestart = false;
  const scaffold = !!document.getElementById('ob-scaffold-files')?.checked;

  if(_onboardMode!=='files'){
    // Apply form values into _rawConfig and save
    if(!_rawConfig) await loadRawConfigForOnboard();
    applyOnboardToRawConfig();

    const r = await fetch('/api/config/raw',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(_rawConfig)});
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'保存配置失败');
    requiresRestart = !!d.requires_restart;
    if(fin){
      fin.innerHTML += `<div class="context-note">配置已保存。${d.requires_restart?'部分修改需要重启后生效。':''}</div>`;
    }
  }

  if(scaffold){
    const scaffoldResult = await runOnboardCreateWorkspaceFiles(true);
    if(!scaffoldResult){
      throw new Error('工作区文件生成失败，请检查下方错误信息后重试');
    }
  }

  // Mark done
  try{ await fetch('/api/onboard/mark-done',{method:'POST'}); }catch(e){}

  // Refresh status in background
  loadStatus();

  return { requires_restart: requiresRestart, scaffolded: scaffold };
}

function applyOnboardToRawConfig(){
  if(!_rawConfig) return;

  // Provider
  const providerId = document.getElementById('ob-provider')?.value || 'openrouter';
  const model = (document.getElementById('ob-model')?.value || '').trim();
  const apiKey = (document.getElementById('ob-api-key')?.value || '').trim();
  const apiUrl = (document.getElementById('ob-api-url')?.value || '').trim();

  if(providerId==='compatible'){
    _rawConfig.default_provider = apiUrl ? `custom:${apiUrl}` : 'custom:';
    if(apiUrl) _rawConfig.api_url = apiUrl;
  } else {
    _rawConfig.default_provider = providerId;
  }
  if(model) _rawConfig.default_model = model;
  if(apiKey) _rawConfig.api_key = apiKey;

  // Channels
  if(!_rawConfig.channels_config) _rawConfig.channels_config = {};

  // OneBot v11
  const onebotOn = !!document.getElementById('ob-ch-onebot')?.checked;
  if(!onebotOn){
    _rawConfig.channels_config.onebot_v11 = null;
  } else {
    const prev = _rawConfig.channels_config.onebot_v11 || {};
    const tokenInp = (document.getElementById('ob-onebot-token')?.value || '').trim();
    const allowedUsers = str2arr(document.getElementById('ob-onebot-allowed-users')?.value || '');
    const allowedGroups = str2arr(document.getElementById('ob-onebot-allowed-groups')?.value || '');
    _rawConfig.channels_config.onebot_v11 = {
      api_url: (document.getElementById('ob-onebot-api-url')?.value || prev.api_url || 'http://127.0.0.1:3000').trim(),
      access_token: tokenInp ? tokenInp : (prev.access_token ?? null),
      listen_host: (document.getElementById('ob-onebot-listen-host')?.value || prev.listen_host || '0.0.0.0').trim(),
      listen_port: Number(document.getElementById('ob-onebot-listen-port')?.value || prev.listen_port || 8096) || 8096,
      allowed_users: allowedUsers,
      allowed_groups: allowedGroups,
      require_at_in_group: !!document.getElementById('ob-onebot-require-at')?.checked,
      admin_users: Array.isArray(prev.admin_users) ? prev.admin_users : [],
      admin_only_tools: Array.isArray(prev.admin_only_tools) ? prev.admin_only_tools : [],
      command_external_network_access: (document.getElementById('ob-onebot-net-access')?.value || prev.command_external_network_access || 'off').trim() || 'off',
      non_admin_context_file: (prev.non_admin_context_file || 'NON_ADMIN.md').trim() || 'NON_ADMIN.md',
    };
  }

  // Telegram
  const tgOn = !!document.getElementById('ob-ch-telegram')?.checked;
  if(!tgOn){
    _rawConfig.channels_config.telegram = null;
  } else {
    const prev = _rawConfig.channels_config.telegram || {};
    const tokenInp = (document.getElementById('ob-telegram-token')?.value || '').trim();
    _rawConfig.channels_config.telegram = {
      bot_token: tokenInp || prev.bot_token || '',
      allowed_users: str2arr(document.getElementById('ob-telegram-allowed')?.value || ''),
    };
  }

  // Discord
  const dsOn = !!document.getElementById('ob-ch-discord')?.checked;
  if(!dsOn){
    _rawConfig.channels_config.discord = null;
  } else {
    const prev = _rawConfig.channels_config.discord || {};
    const tokenInp = (document.getElementById('ob-discord-token')?.value || '').trim();
    const gid = (document.getElementById('ob-discord-guild')?.value || '').trim();
    _rawConfig.channels_config.discord = {
      bot_token: tokenInp || prev.bot_token || '',
      guild_id: gid || prev.guild_id || null,
      allowed_users: prev.allowed_users || [],
      listen_to_bots: !!prev.listen_to_bots,
      mention_only: !!prev.mention_only,
    };
  }

  // Webhook
  const whOn = !!document.getElementById('ob-ch-webhook')?.checked;
  if(!whOn){
    _rawConfig.channels_config.webhook = null;
  } else {
    const prev = _rawConfig.channels_config.webhook || {};
    const secretInp = (document.getElementById('ob-webhook-secret')?.value || '').trim();
    _rawConfig.channels_config.webhook = {
      port: Number(document.getElementById('ob-webhook-port')?.value || prev.port || 8080) || 8080,
      secret: secretInp ? secretInp : (prev.secret ?? null)
    };
  }

  // Tunnel
  if(!_rawConfig.tunnel) _rawConfig.tunnel = {provider:'none'};
  const tProvider = document.getElementById('ob-tunnel-provider')?.value || 'none';
  _rawConfig.tunnel.provider = tProvider;
  if(tProvider==='cloudflare'){
    const tok = (document.getElementById('ob-tunnel-cf-token')?.value || '').trim();
    const prev = _rawConfig.tunnel.cloudflare || {};
    _rawConfig.tunnel.cloudflare = { token: tok || prev.token || '' };
    _rawConfig.tunnel.tailscale = null; _rawConfig.tunnel.ngrok=null; _rawConfig.tunnel.custom=null;
  } else if(tProvider==='tailscale'){
    const prev = _rawConfig.tunnel.tailscale || {};
    _rawConfig.tunnel.tailscale = {
      funnel: !!document.getElementById('ob-tunnel-ts-funnel')?.checked,
      hostname: (document.getElementById('ob-tunnel-ts-host')?.value || '').trim() || prev.hostname || null,
    };
    _rawConfig.tunnel.cloudflare = null; _rawConfig.tunnel.ngrok=null; _rawConfig.tunnel.custom=null;
  } else if(tProvider==='ngrok'){
    const tok = (document.getElementById('ob-tunnel-ngrok-token')?.value || '').trim();
    const prev = _rawConfig.tunnel.ngrok || {};
    _rawConfig.tunnel.ngrok = {
      auth_token: tok || prev.auth_token || '',
      domain: (document.getElementById('ob-tunnel-ngrok-domain')?.value || '').trim() || prev.domain || null,
    };
    _rawConfig.tunnel.cloudflare = null; _rawConfig.tunnel.tailscale=null; _rawConfig.tunnel.custom=null;
  } else if(tProvider==='custom'){
    _rawConfig.tunnel.custom = {
      start_command: (document.getElementById('ob-tunnel-custom-cmd')?.value || '').trim(),
      health_url: (document.getElementById('ob-tunnel-custom-health')?.value || '').trim() || null,
      url_pattern: (document.getElementById('ob-tunnel-custom-pattern')?.value || '').trim() || null,
    };
    _rawConfig.tunnel.cloudflare = null; _rawConfig.tunnel.tailscale=null; _rawConfig.tunnel.ngrok=null;
  } else {
    _rawConfig.tunnel.cloudflare = null; _rawConfig.tunnel.tailscale=null; _rawConfig.tunnel.ngrok=null; _rawConfig.tunnel.custom=null;
  }

  // Autonomy + secrets + composio
  if(!_rawConfig.autonomy) _rawConfig.autonomy = {};
  _rawConfig.autonomy.level = document.getElementById('ob-autonomy-level')?.value || 'supervised';
  if(!_rawConfig.secrets) _rawConfig.secrets = {};
  _rawConfig.secrets.encrypt = !!document.getElementById('ob-secrets-encrypt')?.checked;
  if(!_rawConfig.composio) _rawConfig.composio = {};
  _rawConfig.composio.enabled = !!document.getElementById('ob-composio-enabled')?.checked;
  const composioKey = (document.getElementById('ob-composio-api-key')?.value || '').trim();
  if(composioKey) _rawConfig.composio.api_key = composioKey;
  _rawConfig.composio.entity_id = (document.getElementById('ob-composio-entity')?.value || 'default').trim() || 'default';

  // Hardware
  if(!_rawConfig.hardware) _rawConfig.hardware = {};
  _rawConfig.hardware.enabled = !!document.getElementById('ob-hw-enabled')?.checked;
  _rawConfig.hardware.transport = document.getElementById('ob-hw-transport')?.value || 'None';
  _rawConfig.hardware.serial_port = (document.getElementById('ob-hw-serial-port')?.value || '').trim() || null;
  _rawConfig.hardware.baud_rate = Number(document.getElementById('ob-hw-baud')?.value || 115200) || 115200;
  _rawConfig.hardware.probe_target = (document.getElementById('ob-hw-probe-target')?.value || '').trim() || null;
  _rawConfig.hardware.workspace_datasheets = !!document.getElementById('ob-hw-datasheets')?.checked;

  // Memory
  if(!_rawConfig.memory) _rawConfig.memory = {};
  _rawConfig.memory.backend = document.getElementById('ob-mem-backend')?.value || 'sqlite';
  _rawConfig.memory.auto_save = !!document.getElementById('ob-mem-autosave')?.checked;
  _rawConfig.memory.conversation_retention_days = Number(document.getElementById('ob-mem-retention')?.value || 30) || 30;
  if(!_rawConfig.agent) _rawConfig.agent = {};
  _rawConfig.agent.max_history_messages = Number(document.getElementById('ob-agent-max-history')?.value || 100) || 100;
}

window.onboardCreateWorkspaceFiles = async function(silent){
  return await runOnboardCreateWorkspaceFiles(!!silent);
};

async function runOnboardCreateWorkspaceFiles(silent){
  const fin = document.getElementById('ob-finish-result');
  try{
    const payload = {
      user_name: (document.getElementById('ob-user-name')?.value || '').trim(),
      timezone: (document.getElementById('ob-timezone')?.value || '').trim(),
      agent_name: (document.getElementById('ob-agent-name')?.value || '').trim(),
      communication_style: onboardStyleFromPreset(),
      communication_language: onboardLanguageFromPreset(),
      mark_done: false,
      refresh_personalization: _onboardMode !== 'files',
    };
    const r = await fetch('/api/onboard/scaffold',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
    const d = await r.json().catch(()=>({}));
    if(!r.ok) throw new Error(d.error||'生成失败');
    if(fin){
      const created = Array.isArray(d.created) ? d.created : [];
      const skipped = Array.isArray(d.skipped) ? d.skipped : [];
      const updated = Array.isArray(d.personalization_updated) ? d.personalization_updated : [];
      const workspaceDir = typeof d.workspace_dir === 'string' ? d.workspace_dir.trim() : '';
      const pathPart = workspaceDir ? ` 写入目录：<code>${esc(workspaceDir)}</code>。` : '';
      const updatedPart = updated.length ? `，个性化更新 ${updated.length} 个` : '';
      fin.innerHTML += `<div class="context-note">工作区文件已处理：创建 ${created.length} 个，跳过 ${skipped.length} 个${updatedPart}。${pathPart}</div>`;
    }
    _onboardStatus = await fetchOnboardStatus();
    renderOnboardChecklist();
    return d;
  }catch(e){
    if(!silent) alert('生成工作区文件失败：'+(e.message||e));
    if(fin) fin.innerHTML += `<div class="context-note">生成工作区文件失败：${esc(e.message||String(e))}</div>`;
    return null;
  }
}

async function checkOnboardAutoShow(){
  // Avoid auto-show if already open
  if(onboardIsActive()) return;
  const st = await fetchOnboardStatus();
  if(st && st.needs_setup){
    _onboardMode = 'full';
    await window.openOnboardWizard(false, 'full');
  }
}

loadStatus();
checkOnboardAutoShow();
