(() => {
  'use strict';
  const $ = id => document.getElementById(id), bridge = window.__TAURI__;
  let busy = false, maintaining = false, pendingUpdate = false, taskReady = false, ready = false, video = null, output = '', currentPhase = 'idle';
  let paused = false, pausePending = false, processing = false, currentRequest = null;
  const logs = [];
  function log(line) {
    logs.push(line); if (logs.length > 250) logs.shift();
    $('logs').textContent = logs.join('\n'); $('logs').scrollTop = $('logs').scrollHeight;
  }
  function error(message) { $('error').hidden = !message; $('error').textContent = message || ''; }
  function progress(value, indeterminate = false) {
    const number = Math.max(0, Math.min(100, Number(value) || 0));
    $('progress-track').classList.toggle('indeterminate', indeterminate);
    $('progress-fill').style.width = `${number}%`;
    $('progress-number').textContent = indeterminate ? '处理中' : `${Math.round(number)}%`;
    if (indeterminate) $('progress-track').removeAttribute('aria-valuenow');
    else $('progress-track').setAttribute('aria-valuenow', String(Math.round(number)));
  }
  function controls() {
    const locked = busy || maintaining || paused;
    const mode = $('download-mode').value, withSubtitles = mode.includes('subtitles'), hasVideo = mode.startsWith('video');
    $('video-url').disabled = locked; $('parse-button').disabled = locked || !ready;
    $('parse-button').textContent = currentPhase === 'parsing' ? '解析中…' : '解析视频';
    $('choose-folder').disabled = locked; $('quality').disabled = locked || !video;
    $('container').disabled = locked; $('download-button').disabled = locked || !ready || !video || (withSubtitles && $('subtitle-language').value === '');
    $('download-mode').disabled = locked;
    for (const option of $('download-mode').options) option.disabled = Boolean(video) && !video.subtitles.length && option.value.includes('subtitles');
    $('subtitle-language').disabled = locked || !video || !video.subtitles.length;
    $('check-updates').disabled = locked; $('update-tools').disabled = locked;
    $('cancel-button').disabled = pausePending || (!paused && (!busy || !taskReady));
    $('pause-button').disabled = maintaining || pausePending || mode === 'subtitles' || (!paused && (!busy || !taskReady || currentPhase !== 'downloading' || processing));
    $('pause-button').textContent = paused ? '继续下载' : pausePending ? '正在暂停…' : '暂停下载';
    $('download-text').textContent = currentPhase === 'complete' ? '再次下载' : '开始下载';
    $('close-note').textContent = busy || paused ? '关闭窗口会结束当前任务，保留可续传文件。' : '视频和音频均支持暂停，继续时尝试断点续传。';
    $('quality-option').hidden = !hasVideo; $('container-option').hidden = !hasVideo;
    $('subtitle-option').hidden = !withSubtitles; $('audio-format-option').hidden = hasVideo;
    $('audio-format-label').textContent = mode === 'subtitles' ? '字幕格式' : '音频格式';
    $('fixed-format-value').textContent = mode === 'subtitles' ? 'SRT' : 'MP3';
    $('download-options').classList.toggle('compact-options', !hasVideo);
    const notes = {
      video: '默认保留最佳画质，不进行转码。已下载的同名文件不会被覆盖。',
      video_subtitles: '保存视频和所选语言的 SRT 字幕，字幕为独立文件。',
      audio: '仅下载 MP3 音频。已下载的同名文件不会被覆盖。',
      audio_subtitles: '仅下载 MP3 音频和所选语言的 SRT 字幕，不下载视频。',
      subtitles: '仅保存所选语言的 SRT 字幕文件。',
    };
    $('option-note').textContent = withSubtitles && video && !video.subtitles.length
      ? '当前视频没有可用字幕，请选择“视频”或“音频”模式。' : notes[mode];
  }
  $('download-mode').addEventListener('change', controls);
  $('subtitle-language').addEventListener('change', controls);
  function phase(name, title, description) {
    currentPhase = name; $('state-text').textContent = title;
    $('state-label').dataset.active = String(!['idle', 'error'].includes(name));
    $('progress-title').textContent = title;
    if (description) $('status-description').textContent = description;
    controls();
  }
  function duration(seconds) {
    if (!Number.isFinite(seconds)) return '时长未知';
    const n = Math.floor(seconds), h = Math.floor(n / 3600), m = Math.floor(n / 60) % 60, s = n % 60;
    return h ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
  }
  function speed(n) {
    if (!Number.isFinite(n) || n < 0) return '';
    return n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB/s` : `${Math.round(n / 1024)} KB/s`;
  }
  function setOutput(path) {
    output = path; $('folder-value').textContent = path; $('folder-value').title = path;
    $('settings-path').textContent = path;
  }
  function showVideo(data) {
    video = data; $('video-title').textContent = data.title; $('preview-copy').classList.remove('empty');
    $('channel').textContent = data.channel || 'YouTube'; $('duration').textContent = duration(data.duration);
    $('duration-tag').textContent = duration(data.duration); $('duration-tag').hidden = !Number.isFinite(data.duration);
    $('metadata').hidden = false;
    $('description').textContent = data.description || '选择画质和保存位置，即可开始下载。';
    $('resolution').textContent = data.heights.length ? `最高 ${data.heights[0]}p` : '';
    $('quality').replaceChildren(new Option('最佳画质', 'best'), ...data.heights.map(h => new Option(`${h}p`, String(h))));
    setSubtitles(data.subtitles);
    $('thumbnail').hidden = true; $('thumbnail-empty').hidden = false; $('thumbnail').src = data.thumbnail;
  }
  function setSubtitles(tracks) {
    $('subtitle-language').replaceChildren(new Option(!video ? '解析后选择字幕' : tracks.length ? '请选择字幕语言' : '当前视频没有可用字幕', ''));
    for (const automatic of [false, true]) {
      const group = document.createElement('optgroup'); group.label = automatic ? '自动字幕' : '人工字幕';
      tracks.forEach((track, index) => {
        if (track.automatic === automatic) group.append(new Option(`${track.name} (${track.language})${automatic ? ' / 自动字幕' : ''}`, String(index)));
      });
      if (group.children.length) $('subtitle-language').append(group);
    }
    if (tracks.length) {
      let preferred = tracks.findIndex(track => !track.automatic && /^en(?:-|$)/i.test(track.language));
      if (preferred < 0) preferred = tracks.findIndex(track => /^en(?:-|$)/i.test(track.language));
      $('subtitle-language').value = String(preferred >= 0 ? preferred : 0);
    }
  }
  $('thumbnail').addEventListener('load', () => { $('thumbnail').hidden = false; $('thumbnail-empty').hidden = true; });
  $('thumbnail').addEventListener('error', () => { $('thumbnail').hidden = true; $('thumbnail-empty').hidden = false; });
  $('video-url').addEventListener('input', () => {
    if (!video) return;
    video = null; $('video-title').textContent = '链接已更改，请重新解析'; $('preview-copy').classList.add('empty');
    setSubtitles([]);
    $('metadata').hidden = true; $('thumbnail').removeAttribute('src'); $('thumbnail').hidden = true;
    $('thumbnail-empty').hidden = false; $('duration-tag').hidden = true;
    $('description').textContent = '解析当前链接后，才能开始下载。';
    $('quality').replaceChildren(new Option('最佳画质', 'best'));
    progress(0); error(''); phase('idle', '等待解析', '链接已更改，请重新解析当前视频。');
  });
  function handleFailure(cause) {
    const message = String(cause);
    if (message === '任务已暂停') { paused = true; phase('paused', '已暂停', '下载已暂停，点击“继续下载”尝试断点续传。'); }
    else if (message === '任务已取消') phase('cancelled', '已取消', '可重新解析或下载，已下载的临时文件会保留。');
    else { error(message); phase('error', '处理失败', '请查看错误提示，可展开运行日志了解具体原因。'); }
    log(message);
  }
  $('parse-form').addEventListener('submit', async event => {
    event.preventDefault(); if (busy || maintaining || paused || !ready) return;
    busy = true; taskReady = false; video = null; error(''); progress(0, true); $('speed-eta').textContent = '';
    setSubtitles([]);
    phase('parsing', '正在解析', '正在读取视频信息和可用画质…');
    try {
      const data = await bridge.core.invoke('inspect_video', { url: $('video-url').value });
      showVideo(data); $('video-url').value = data.url;
      phase('ready', '可以下载', data.subtitles.length ? '视频信息已就绪，选择下载内容后开始下载。' : '当前视频没有可用字幕，可下载视频或音频。');
    } catch (cause) { handleFailure(cause); }
    finally { busy = false; progress(0); controls(); }
  });
  async function startDownload(request) {
    currentRequest = request; paused = false; processing = false;
    busy = true; taskReady = false; error(''); progress(0, true); $('speed-eta').textContent = '';
    phase('downloading', '正在下载', request.mode === 'subtitles' ? '正在准备所选字幕…' : request.mode.startsWith('audio') ? '正在准备音频…' : '正在准备音视频流，进度按当前流显示。');
    try {
      const result = await bridge.core.invoke('download_video', request);
      const subtitles = result.subtitle_files.length ? `，含 ${result.subtitle_files.length} 份字幕` : '';
      const file = result.file || result.subtitle_files[0];
      progress(100); phase('complete', '下载完成', file ? `已保存：${file.split(/[\\/]/).pop()}${result.file ? subtitles : ''}` : `文件已保存到 ${output}`);
    } catch (cause) { handleFailure(cause); if (!paused) progress(0); }
    finally { busy = false; pausePending = false; if (!paused) currentRequest = null; $('speed-eta').textContent = ''; controls(); }
  }
  $('download-button').addEventListener('click', () => {
    if (busy || maintaining || paused || !video || !ready) return;
    const mode = $('download-mode').value;
    const track = mode.includes('subtitles') && $('subtitle-language').value !== '' ? video.subtitles[Number($('subtitle-language').value)] : null;
    if (mode.includes('subtitles') && !track) { error('当前视频没有可下载的所选字幕，请重新解析或切换下载模式。'); return; }
    startDownload({ url: video.url, height: !mode.startsWith('video') || $('quality').value === 'best' ? null : Number($('quality').value), container: $('container').value,
      mode, subtitle: track ? { language: track.language, automatic: track.automatic } : null });
  });
  $('pause-button').addEventListener('click', async () => {
    if (paused && currentRequest) { await startDownload(currentRequest); return; }
    if (!busy || !taskReady || processing || pausePending || currentPhase !== 'downloading') return;
    pausePending = true; controls();
    try { await bridge.core.invoke('pause_task'); }
    catch (cause) { if (busy) error(String(cause)); }
    finally { pausePending = false; controls(); }
  });
  $('cancel-button').addEventListener('click', async () => {
    if (paused) { paused = false; currentRequest = null; progress(0); phase('cancelled', '已取消', '任务已取消，已下载的临时文件会保留。'); return; }
    if (!busy) return;
    $('cancel-button').disabled = true; $('status-description').textContent = '正在取消任务…';
    try { await bridge.core.invoke('cancel_task'); }
    catch (cause) { error(String(cause)); if (busy) $('cancel-button').disabled = false; }
  });
  $('choose-folder').addEventListener('click', async () => {
    if (busy || maintaining || paused) return;
    try { const path = await bridge.core.invoke('choose_directory'); if (path) setOutput(path); }
    catch (cause) { error(String(cause)); }
  });
  $('open-folder').addEventListener('click', async () => {
    try { await bridge.core.invoke('open_directory'); } catch (cause) { error(String(cause)); }
  });
  $('settings').addEventListener('click', () => $('settings-dialog').showModal());
  $('close-settings').addEventListener('click', () => $('settings-dialog').close());
  function applySettings(settings) {
    setOutput(settings.output); ready = settings.ready;
    $('tools-status').textContent = ready ? '下载组件已就绪' : '下载组件需要安装';
    $('settings-tools').textContent = settings.message; controls();
  }
  async function installComponents(update = false) {
    if (busy || maintaining || paused) return;
    maintaining = true; pendingUpdate = update; error('');
    $('tools-panel').hidden = false; $('install-tools').hidden = true;
    $('tools-title').textContent = update ? '正在更新下载组件' : '正在安装缺失组件';
    $('tools-detail').textContent = '正在连接官方发布源…'; $('tools-progress').removeAttribute('value');
    controls();
    try {
      applySettings(await bridge.core.invoke('install_tools', { update }));
      if (!ready) throw new Error('组件仍不完整，请重试安装。');
      $('tools-panel').hidden = true; $('update-tools').hidden = true;
      $('update-result').textContent = update ? '组件更新已完成。' : '下载组件已就绪。';
    } catch (cause) {
      $('tools-title').textContent = update ? '组件更新失败' : '组件安装失败';
      $('tools-detail').textContent = String(cause); $('tools-progress').hidden = true;
      $('install-tools').textContent = update ? '重试更新' : '重试安装'; $('install-tools').hidden = false;
      $('tools-status').textContent = ready ? '更新失败，可继续使用现有组件' : '组件安装失败，请重试';
      $('update-result').textContent = String(cause); log(String(cause));
    } finally { maintaining = false; controls(); }
  }
  $('install-tools').addEventListener('click', () => installComponents(pendingUpdate));
  $('update-tools').addEventListener('click', () => { $('settings-dialog').close(); installComponents(true); });
  $('check-updates').addEventListener('click', async () => {
    if (busy || maintaining || paused) return;
    maintaining = true; $('update-tools').hidden = true; $('update-result').textContent = '正在查询官方组件版本…'; controls();
    try {
      const updates = await bridge.core.invoke('check_tools_updates');
      $('update-result').textContent = updates.map(item => `${item.name}：${item.installed}\n最新版本：${item.latest}${item.available ? '，可更新' : '，无需更新'}`).join('\n\n');
      $('update-tools').hidden = !updates.some(item => item.available);
    } catch (cause) { $('update-result').textContent = `检查失败：${String(cause)}`; }
    finally { maintaining = false; $('tools-status').textContent = ready ? '下载组件已就绪' : '下载组件需要安装'; controls(); }
  });
  async function init() {
    if (!bridge) { error('请通过 Handy-ytget 桌面程序打开此界面。'); $('tools-status').textContent = '桌面连接不可用'; controls(); return; }
    try {
      await bridge.event.listen('component-event', ({ payload }) => {
        if (!maintaining) return;
        $('tools-detail').textContent = payload.message; $('settings-tools').textContent = payload.message;
        $('tools-status').textContent = payload.message; $('tools-progress').hidden = false;
        if (Number.isFinite(payload.percent)) $('tools-progress').value = Math.max(0, Math.min(100, payload.percent));
        else $('tools-progress').removeAttribute('value');
      });
      await bridge.event.listen('download-event', ({ payload }) => {
        if (payload.kind === 'log') { log(payload.message); return; }
        if (payload.kind === 'started' && busy) { taskReady = true; controls(); return; }
        if (!busy || currentPhase !== 'downloading') return;
        if (payload.kind === 'progress') {
          processing = false; controls();
          progress(payload.percent, payload.percent === null);
          $('speed-eta').textContent = [speed(payload.speed), Number.isFinite(payload.eta) ? `剩余 ${duration(payload.eta)}` : ''].filter(Boolean).join(' / ');
          $('status-description').textContent = payload.status === 'finished' ? '当前流已下载完毕，正在继续处理音视频…' : '正在下载音视频流，进度按当前流显示。';
        }
        if (payload.kind === 'processing') { processing = true; controls(); progress(0, true); $('progress-title').textContent = payload.message; $('status-description').textContent = '正在处理并保存文件，此阶段请稍候。'; $('speed-eta').textContent = ''; }
      });
      applySettings(await bridge.core.invoke('get_settings'));
      if (!ready) await installComponents();
    } catch (cause) { error(String(cause)); $('tools-status').textContent = '初始化失败'; controls(); }
  }
  controls(); init();
})();
