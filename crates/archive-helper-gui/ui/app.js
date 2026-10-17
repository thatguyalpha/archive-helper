// Archive Helper UI Client
// Safe Tauri API resolvers
function getInvoke() {
  return window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke;
}

function getListen() {
  return window.__TAURI__?.event?.listen || window.__TAURI__?.listen;
}

async function invoke(cmd, args) {
  const inv = getInvoke();
  if (inv) {
    return await inv(cmd, args);
  }
  console.warn('Tauri invoke unavailable:', cmd);
}

async function listen(event, cb) {
  const lis = getListen();
  if (lis) {
    return await lis(event, cb);
  }
  console.warn('Tauri listen unavailable:', event);
}

// Disable default browser right-click context menu (Back, Reload, Inspect, etc.)
document.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  return false;
});

// Disable browser navigation / refresh shortcuts (F5, Ctrl+R)
document.addEventListener('keydown', (e) => {
  if (e.key === 'F5' || (e.ctrlKey && (e.key === 'r' || e.key === 'R'))) {
    e.preventDefault();
  }
});

let currentSettings = {};
let customZipDestination = null;
let customUnzipDestination = null;
let pendingZipPaths = [];
let isOperationRunning = false;

// DOM Elements
const dropZip = document.getElementById('dropZip');
const btnFolderZip = document.getElementById('btnFolderZip');
const chkDeleteZip = document.getElementById('chkDeleteZip');
const chkFastZip = document.getElementById('chkFastZip');
const chkPredictExt = document.getElementById('chkPredictExt');
const chkKeepMtimeZip = document.getElementById('chkKeepMtimeZip');
const chkSingleFolderZip = document.getElementById('chkSingleFolderZip');

const dropUnzip = document.getElementById('dropUnzip');
const btnFolderUnzip = document.getElementById('btnFolderUnzip');
const chkDeleteUnzip = document.getElementById('chkDeleteUnzip');
const chkFastUnzip = document.getElementById('chkFastUnzip');
const chkKeepTopDir = document.getElementById('chkKeepTopDir');
const chkKeepMtimeUnzip = document.getElementById('chkKeepMtimeUnzip');

const btnUnitMode = document.getElementById('btnUnitMode');
const unitTop = document.getElementById('unitTop');
const unitBottom = document.getElementById('unitBottom');
const btnAdmin = document.getElementById('btnAdmin');
const btnAlwaysOnTop = document.getElementById('btnAlwaysOnTop');
const btnExit = document.getElementById('btnExit');

const arcBytes = document.getElementById('arcBytes');
const arcFiles = document.getElementById('arcFiles');
const progressArcsGroup = document.getElementById('progressArcsGroup');
const badgeCompleted = document.getElementById('badgeCompleted');
const badgeCircle = document.getElementById('badgeCircle');
const txtCounter = document.getElementById('txtCounter');
const panelSummary = document.getElementById('panelSummary');
const txtSummaryBytes = document.getElementById('txtSummaryBytes');
const txtSummaryFiles = document.getElementById('txtSummaryFiles');

const globalProgressTrigger = document.getElementById('globalProgressTrigger');
const popupQueue = document.getElementById('popupQueue');
const queueItemsList = document.getElementById('queueItemsList');
const btnFollow = document.getElementById('btnFollow');
const txtFollow = document.getElementById('txtFollow');

const singleZipModal = document.getElementById('singleZipModal');
const inputZipName = document.getElementById('inputZipName');
const msgZipValidation = document.getElementById('msgZipValidation');
const btnModalCancel = document.getElementById('btnModalCancel');
const btnModalSubmit = document.getElementById('btnModalSubmit');

const OUTER_CIRCUMFERENCE = 124.407;
const INNER_CIRCUMFERENCE = 98.646;
const QUEUE_OUTER_CIRC = 55.92; // 2 * PI * 8.9
const QUEUE_INNER_CIRC = 42.72; // 2 * PI * 6.8

let queueItems = [];
let popupCloseTimeout = null;
let isQueuePinned = false;
let grandTotalBytes = 0;
let grandTotalFiles = 0;
let isFollowEnabled = true;
let lastScrollTime = 0;

// Format Bytes
function formatBytes(bytes, useBinary) {
  if (bytes === 0) return '0 B';
  const k = useBinary ? 1024 : 1000;
  const sizes = useBinary
    ? ['B', 'KiB', 'MiB', 'GiB', 'TiB']
    : ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

// 1. Initialize Settings
async function initSettings() {
  try {
    console.log('initSettings starting...');
    const loaded = await invoke('load_settings');
    console.log('initSettings loaded:', loaded);
    currentSettings = loaded || {};

    chkDeleteZip.checked = currentSettings.DeleteZip ?? true;
    chkFastZip.checked = currentSettings.FastZip ?? false;
    chkPredictExt.checked = currentSettings.PredictExt ?? true;
    chkKeepMtimeZip.checked = currentSettings.KeepMtimeZip ?? true;
    chkSingleFolderZip.checked = currentSettings.SingleFolderZip ?? false;

    chkDeleteUnzip.checked = currentSettings.DeleteUnzip ?? true;
    chkFastUnzip.checked = currentSettings.FastUnzip ?? false;
    chkKeepTopDir.checked = currentSettings.KeepTopDir ?? true;
    chkKeepMtimeUnzip.checked = currentSettings.KeepMtimeUnzip ?? true;

    updateUnitModeDisplay(currentSettings.UseBinaryUnits ?? true);
    updateAlwaysOnTopDisplay(currentSettings.AlwaysOnTop ?? false);

    // Check Admin Elevation State
    const isAdmin = await invoke('is_running_as_admin');
    if (isAdmin) {
      btnAdmin.classList.add('is-admin');
    } else {
      btnAdmin.classList.remove('is-admin');
    }

    updateFollowButtonState();
  } catch (e) {
    console.error('Failed to load settings:', e);
    currentSettings = {};
    updateFollowButtonState();
  }
}

const THUMBTACK_ON = "M160 96C160 78.3 174.3 64 192 64L448 64C465.7 64 480 78.3 480 96C480 113.7 465.7 128 448 128L418.5 128L428.8 262.1C465.9 283.3 494.6 318.5 507 361.8L510.8 375.2C513.6 384.9 511.6 395.2 505.6 403.3C499.6 411.4 490 416 480 416L160 416C150 416 140.5 411.3 134.5 403.3C128.5 395.3 126.5 384.9 129.3 375.2L133 361.8C145.4 318.5 174 283.3 211.2 262.1L221.5 128L192 128C174.3 128 160 113.7 160 96zM288 464L352 464L352 576C352 593.7 337.7 608 320 608C302.3 608 288 593.7 288 576L288 464z";
const THUMBTACK_OFF = "M73 39.1C63.6 29.7 48.4 29.7 39.1 39.1C29.8 48.5 29.7 63.7 39 73.1L567 601.1C576.4 610.5 591.6 610.5 600.9 601.1C610.2 591.7 610.3 576.5 600.9 567.2L449.8 416L480 416C490 416 499.5 411.3 505.5 403.3C511.5 395.3 513.5 384.9 510.7 375.2L507 361.8C494.6 318.5 466 283.3 428.8 262.1L418.5 128L448 128C465.7 128 480 113.7 480 96C480 78.3 465.7 64 448 64L192 64C184.6 64 177.9 66.5 172.5 70.6L222.1 120.3L217.3 183.4L73 39.1zM314.2 416L181.7 283.6C159 304.1 141.9 331 133 361.9L129.2 375.3C126.4 385 128.4 395.3 134.4 403.4C140.4 411.5 150 416 160 416L314.2 416zM288 576C288 593.7 302.3 608 320 608C337.7 608 352 593.7 352 576L352 464L288 464L288 576z";

function updateAlwaysOnTopDisplay(enabled) {
  const path = document.getElementById('pathThumbtack');
  if (path) {
    path.setAttribute('d', enabled ? THUMBTACK_ON : THUMBTACK_OFF);
  }
  btnAlwaysOnTop.classList.toggle('active', enabled);
}

function updateUnitModeDisplay(isBinary) {
  unitTop.textContent = isBinary ? 'MiB' : 'MB';
  unitBottom.textContent = isBinary ? 'GiB' : 'GB';
}

function persistSettings() {
  currentSettings.DeleteZip = chkDeleteZip.checked;
  currentSettings.FastZip = chkFastZip.checked;
  currentSettings.PredictExt = chkPredictExt.checked;
  currentSettings.KeepMtimeZip = chkKeepMtimeZip.checked;
  currentSettings.SingleFolderZip = chkSingleFolderZip.checked;

  currentSettings.DeleteUnzip = chkDeleteUnzip.checked;
  currentSettings.FastUnzip = chkFastUnzip.checked;
  currentSettings.KeepTopDir = chkKeepTopDir.checked;
  currentSettings.KeepMtimeUnzip = chkKeepMtimeUnzip.checked;

  invoke('save_settings', { settings: currentSettings }).catch(console.error);
}

// Attach Checkbox Listeners
[
  chkDeleteZip, chkFastZip, chkPredictExt, chkKeepMtimeZip, chkSingleFolderZip,
  chkDeleteUnzip, chkFastUnzip, chkKeepTopDir, chkKeepMtimeUnzip,
].forEach(chk => chk.addEventListener('change', persistSettings));

// 2. Button Functions (bound globally for inline & event listeners)
window.toggleUnitMode = () => {
  const isBinary = !(currentSettings.UseBinaryUnits ?? true);
  currentSettings.UseBinaryUnits = isBinary;
  updateUnitModeDisplay(isBinary);
  persistSettings();
};

window.toggleAlwaysOnTop = async () => {
  const current = currentSettings.AlwaysOnTop ?? false;
  const next = !current;
  currentSettings.AlwaysOnTop = next;
  updateAlwaysOnTopDisplay(next);
  persistSettings();
  await invoke('set_always_on_top', { enabled: next });
};

window.triggerAdmin = async () => {
  if (!btnAdmin.classList.contains('is-admin')) {
    await invoke('restart_as_admin');
  }
};

window.triggerExit = () => {
  invoke('exit_app');
};

btnUnitMode.addEventListener('click', window.toggleUnitMode);
btnAlwaysOnTop.addEventListener('click', window.toggleAlwaysOnTop);
btnAdmin.addEventListener('click', window.triggerAdmin);
btnExit.addEventListener('click', window.triggerExit);

// Pick Destination Folder Click
btnFolderZip.addEventListener('click', async (e) => {
  e.stopPropagation();
  const folder = await invoke('pick_folder');
  if (folder) {
    customZipDestination = folder;
    btnFolderZip.style.borderColor = '#007ACC';
  }
});

btnFolderUnzip.addEventListener('click', async (e) => {
  e.stopPropagation();
  const folder = await invoke('pick_folder');
  if (folder) {
    customUnzipDestination = folder;
    btnFolderUnzip.style.borderColor = '#28A745';
  }
});

// 3. Drag and Drop Sub-target Hit Testing
function clearDragHighlights() {
  dropZip.classList.remove('drag-over');
  btnFolderZip.classList.remove('drag-target');
  dropUnzip.classList.remove('drag-over');
  btnFolderUnzip.classList.remove('drag-target');
}

function handleDragPosition(x, y) {
  clearDragHighlights();
  const el = document.elementFromPoint(x, y);
  if (!el) return;

  if (btnFolderZip.contains(el)) {
    btnFolderZip.classList.add('drag-target');
  } else if (dropZip.contains(el)) {
    dropZip.classList.add('drag-over');
  } else if (btnFolderUnzip.contains(el)) {
    btnFolderUnzip.classList.add('drag-target');
  } else if (dropUnzip.contains(el)) {
    dropUnzip.classList.add('drag-over');
  }
}

listen('window-drag-enter', (e) => {
  handleDragPosition(e.payload.x, e.payload.y);
});

listen('window-drag-over', (e) => {
  handleDragPosition(e.payload.x, e.payload.y);
});

listen('window-drag-leave', () => {
  clearDragHighlights();
});

listen('window-drag-drop', async (e) => {
  clearDragHighlights();
  const { paths, x, y } = e.payload;
  if (!paths || paths.length === 0) return;

  const el = document.elementFromPoint(x, y);
  if (!el) return;

  if (btnFolderZip.contains(el)) {
    customZipDestination = paths[0];
    btnFolderZip.style.borderColor = '#007ACC';
  } else if (btnFolderUnzip.contains(el)) {
    customUnzipDestination = paths[0];
    btnFolderUnzip.style.borderColor = '#28A745';
  } else if (dropZip.contains(el)) {
    triggerZip(paths);
  } else if (dropUnzip.contains(el)) {
    triggerUnzip(paths);
  }
});

// 4. Trigger Operations
async function triggerZip(paths) {
  if (isOperationRunning) return;

  if (chkSingleFolderZip.checked) {
    pendingZipPaths = paths;
    const defaultName = await invoke('get_default_zip_name', { paths });
    inputZipName.value = defaultName;
    msgZipValidation.classList.add('hidden');
    singleZipModal.classList.remove('hidden');
    inputZipName.focus();
    inputZipName.select();
  } else {
    executeZip(paths, null);
  }
}

async function executeZip(paths, singleName) {
  isOperationRunning = true;
  const isZip = true;
  queueItems = singleName
    ? [{ id: 'item_0', path: singleName, name: `${singleName}.zip`, bytes_processed: 0, total_bytes: 0, files_processed: 0, total_files: 0, completed: false, isZip }]
    : paths.map((p, idx) => ({
        id: `item_${idx}`,
        path: p,
        name: p.split(/[\\/]/).pop() || p,
        bytes_processed: 0,
        total_bytes: 0,
        files_processed: 0,
        total_files: 0,
        completed: false,
        isZip
      }));
  renderQueue();
  resetProgress(isZip);

  try {
    await invoke('start_zip_batch', {
      items: paths,
      destination: customZipDestination,
      deleteOriginal: chkDeleteZip.checked,
      fast: chkFastZip.checked,
      predictExt: chkPredictExt.checked,
      keepMtime: chkKeepMtimeZip.checked,
      singleZipName: singleName,
    });
  } catch (err) {
    console.error('Zip batch failed:', err);
    isOperationRunning = false;
  }
}

async function triggerUnzip(paths) {
  if (isOperationRunning) return;
  isOperationRunning = true;
  const isZip = false;
  queueItems = paths.map((p, idx) => ({
    id: `item_${idx}`,
    path: p,
    name: p.split(/[\\/]/).pop() || p,
    bytes_processed: 0,
    total_bytes: 0,
    files_processed: 0,
    total_files: 0,
    completed: false,
    isZip
  }));
  renderQueue();
  resetProgress(isZip);

  try {
    await invoke('start_unzip_batch', {
      items: paths,
      destination: customUnzipDestination,
      deleteOriginal: chkDeleteUnzip.checked,
      fast: chkFastUnzip.checked,
      keepTopDir: chkKeepTopDir.checked,
      keepMtime: chkKeepMtimeUnzip.checked,
    });
  } catch (err) {
    console.error('Unzip batch failed:', err);
    isOperationRunning = false;
  }
}

// 5. Modal Handlers
btnModalCancel.addEventListener('click', () => {
  singleZipModal.classList.add('hidden');
  pendingZipPaths = [];
});

btnModalSubmit.addEventListener('click', async () => {
  const name = inputZipName.value.trim();
  const isValid = await invoke('validate_archive_name', { name });
  if (!isValid) {
    msgZipValidation.classList.remove('hidden');
    return;
  }
  singleZipModal.classList.add('hidden');
  const paths = pendingZipPaths;
  pendingZipPaths = [];
  executeZip(paths, name);
});

inputZipName.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    btnModalSubmit.click();
  } else if (e.key === 'Escape') {
    btnModalCancel.click();
  }
});

// 6. Queue Flyout Management (Hover & Pin)
function showQueuePopup() {
  if (queueItems.length === 0 && !isOperationRunning) return;
  if (popupCloseTimeout) {
    clearTimeout(popupCloseTimeout);
    popupCloseTimeout = null;
  }
  popupQueue.classList.remove('hidden');
}

function scheduleCloseQueuePopup() {
  if (isQueuePinned) return;
  if (popupCloseTimeout) clearTimeout(popupCloseTimeout);
  popupCloseTimeout = setTimeout(() => {
    if (!isQueuePinned) {
      popupQueue.classList.add('hidden');
    }
  }, 250);
}

// Hover on progress rings cluster
globalProgressTrigger.addEventListener('mouseenter', showQueuePopup);
globalProgressTrigger.addEventListener('mouseleave', scheduleCloseQueuePopup);

// Hover on popup itself
popupQueue.addEventListener('mouseenter', () => {
  if (popupCloseTimeout) {
    clearTimeout(popupCloseTimeout);
    popupCloseTimeout = null;
  }
});
popupQueue.addEventListener('mouseleave', scheduleCloseQueuePopup);

// Click progress rings to toggle pin
globalProgressTrigger.addEventListener('click', (e) => {
  e.stopPropagation();
  if (queueItems.length === 0 && !isOperationRunning) return;
  isQueuePinned = !isQueuePinned;
  if (isQueuePinned) {
    showQueuePopup();
  } else {
    scheduleCloseQueuePopup();
  }
});

// Click outside unpins and closes popup
document.addEventListener('click', (e) => {
  if (isQueuePinned && !popupQueue.contains(e.target) && !globalProgressTrigger.contains(e.target)) {
    isQueuePinned = false;
    popupQueue.classList.add('hidden');
  }
});

function renderQueue() {
  queueItemsList.innerHTML = '';
  queueItems.forEach(item => {
    const el = document.createElement('div');
    el.className = 'queue-item';
    el.id = `qItem_${item.id}`;
    el.innerHTML = `
      <div class="queue-item-ring" style="width: 20px; height: 20px; position: relative; flex-shrink: 0;">
        <svg viewBox="0 0 20 20" width="20" height="20" class="q-ring-svg">
          <circle cx="10" cy="10" r="8.9" fill="none" stroke="#262626" stroke-width="2.2" />
          <circle cx="10" cy="10" r="6.8" fill="none" stroke="#241730" stroke-width="2.0" />
          <circle class="q-arc-bytes" cx="10" cy="10" r="8.9" fill="none" stroke="${item.isZip ? '#007ACC' : '#28A745'}" stroke-width="2.2" stroke-linecap="round" stroke-dasharray="${QUEUE_OUTER_CIRC}" stroke-dashoffset="${QUEUE_OUTER_CIRC}" style="transition: stroke-dashoffset 0.08s linear;" />
          <circle class="q-arc-files" cx="10" cy="10" r="6.8" fill="none" stroke="#B026FF" stroke-width="2.0" stroke-linecap="round" stroke-dasharray="${QUEUE_INNER_CIRC}" stroke-dashoffset="${QUEUE_INNER_CIRC}" style="transition: stroke-dashoffset 0.08s linear;" />
        </svg>
        <div class="q-badge-completed" style="display: none; position: absolute; top: 0; left: 0; width: 20px; height: 20px;">
          <svg width="20" height="20" viewBox="0 0 20 20">
            <circle cx="10" cy="10" r="10" class="q-badge-circle" fill="${item.isZip ? '#007ACC' : '#28A745'}" />
            <path class="q-badge-icon" d="M 5,10.2 L 8.4,13.6 L 15,7" stroke="#FFFFFF" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/>
          </svg>
        </div>
      </div>
      <div class="queue-item-name">${item.name}</div>
      <div class="queue-item-stats">
        <span class="q-bytes-text">${item.total_bytes > 0 ? formatBytes(item.total_bytes, currentSettings.UseBinaryUnits ?? true) : ''}</span>
        <span class="q-files-text">${item.total_files > 0 ? `(${item.total_files})` : ''}</span>
      </div>
    `;
    queueItemsList.appendChild(el);
  });
  const txtQueueCount = document.getElementById('txtQueueCount');
  if (txtQueueCount) {
    txtQueueCount.textContent = `${queueItems.length} items`;
  }
}

function updateQueueItemElement(item) {
  const el = document.getElementById(`qItem_${item.id}`);
  if (!el) return;
  const isBinary = currentSettings.UseBinaryUnits ?? true;

  const arcBytes = el.querySelector('.q-arc-bytes');
  const arcFiles = el.querySelector('.q-arc-files');
  const badge = el.querySelector('.q-badge-completed');
  const badgeCircle = el.querySelector('.q-badge-circle');
  const badgeIcon = el.querySelector('.q-badge-icon');
  const ringSvg = el.querySelector('.q-ring-svg');
  const txtBytes = el.querySelector('.q-bytes-text');
  const txtFiles = el.querySelector('.q-files-text');

  if (item.completed) {
    if (ringSvg) ringSvg.style.display = 'none';
    if (badge) badge.style.display = 'block';

    if (item.success === false) {
      if (badgeCircle) badgeCircle.setAttribute('fill', '#DC3545');
      if (badgeIcon) {
        badgeIcon.setAttribute('d', 'M 6,6 L 14,14 M 14,6 L 6,14');
        badgeIcon.setAttribute('stroke', '#FFFFFF');
      }
    } else {
      if (badgeCircle) badgeCircle.setAttribute('fill', item.isZip ? '#007ACC' : '#28A745');
      if (badgeIcon) {
        badgeIcon.setAttribute('d', 'M 5,10.2 L 8.4,13.6 L 15,7');
        badgeIcon.setAttribute('stroke', '#FFFFFF');
      }
    }

    if (txtBytes) txtBytes.textContent = formatBytes(item.total_bytes, isBinary);
    if (txtFiles) txtFiles.textContent = `(${item.total_files})`;
  } else {
    if (ringSvg) ringSvg.style.display = 'block';
    if (badge) badge.style.display = 'none';
    const bRatio = item.total_bytes > 0 ? item.bytes_processed / item.total_bytes : 0;
    const fRatio = item.total_files > 0 ? item.files_processed / item.total_files : 0;
    if (arcBytes) arcBytes.style.strokeDashoffset = QUEUE_OUTER_CIRC * (1 - Math.min(Math.max(bRatio, 0), 1));
    if (arcFiles) arcFiles.style.strokeDashoffset = QUEUE_INNER_CIRC * (1 - Math.min(Math.max(fRatio, 0), 1));
    if (txtBytes) {
      txtBytes.textContent = item.total_bytes > 0
        ? `${formatBytes(item.bytes_processed, isBinary)} / ${formatBytes(item.total_bytes, isBinary)}`
        : '';
    }
    if (txtFiles) {
      txtFiles.textContent = item.total_files > 0
        ? `(${item.files_processed}/${item.total_files})`
        : '';
    }
  }
}

function updateFollowButtonState() {
  if (!btnFollow) return;
  if (isFollowEnabled) {
    btnFollow.classList.add('active');
  } else {
    btnFollow.classList.remove('active');
  }
}

if (btnFollow) {
  btnFollow.addEventListener('click', (e) => {
    e.stopPropagation();
    isFollowEnabled = !isFollowEnabled;
    updateFollowButtonState();
    if (isFollowEnabled) {
      triggerAutoScroll(true);
    }
  });
}

function triggerAutoScroll(forceTeleport = false) {
  if (!isFollowEnabled || queueItems.length === 0 || !queueItemsList) return;

  // 1. Identify all active (non-completed) working operations
  const activeIndices = [];
  for (let i = 0; i < queueItems.length; i++) {
    if (!queueItems[i].completed) {
      if (queueItems[i].bytes_processed > 0 || activeIndices.length === 0) {
        activeIndices.push(i);
      }
    }
  }

  if (activeIndices.length === 0) return;

  // 2. Multithread target:
  // "make it either have the lowest working operation in the middle or have the middle of the working operations in the middle"
  // We pick the middle working operation of the active set to center the active cluster
  const middleIdx = activeIndices[Math.floor(activeIndices.length / 2)];
  const targetItem = queueItems[middleIdx];
  if (!targetItem) return;

  const targetEl = document.getElementById(`qItem_${targetItem.id}`);
  if (!targetEl) return;

  // 3. Calculate position to center the working item vertically in the viewport
  const viewportHeight = queueItemsList.clientHeight;
  const itemHeight = targetEl.clientHeight;
  const targetScrollTop = Math.max(0, targetEl.offsetTop - (viewportHeight / 2) + (itemHeight / 2));

  const currentScrollTop = queueItemsList.scrollTop;
  const delta = Math.abs(currentScrollTop - targetScrollTop);

  // If already centered (within 4px), don't trigger unnecessary scroll
  if (delta < 4 && !forceTeleport) return;

  const now = performance.now();
  const timeSinceLast = now - lastScrollTime;
  lastScrollTime = now;

  // "make it smoothly scroll down instead of teleporting. when its going too fast, then itll teleport."
  const isGoingTooFast = timeSinceLast < 140 || delta > 120;

  if (forceTeleport || isGoingTooFast) {
    queueItemsList.scrollTop = targetScrollTop;
  } else {
    queueItemsList.scrollTo({
      top: targetScrollTop,
      behavior: 'smooth'
    });
  }
}

// 7. Progress & Completed Event Listeners
function resetProgress(isZip) {
  badgeCompleted.style.opacity = '0';
  badgeCompleted.style.pointerEvents = 'none';
  progressArcsGroup.style.opacity = '1';
  arcBytes.style.stroke = isZip ? '#007ACC' : '#28A745';
  badgeCircle.style.backgroundColor = isZip ? '#007ACC' : '#28A745';
  arcBytes.style.strokeDashoffset = OUTER_CIRCUMFERENCE;
  arcFiles.style.strokeDashoffset = INNER_CIRCUMFERENCE;
  txtCounter.textContent = `0/${queueItems.length || 1}`;
  txtSummaryBytes.textContent = '';
  txtSummaryFiles.textContent = '';
  panelSummary.style.display = 'flex';
}

listen('op-init', (e) => {
  const p = e.payload;
  isOperationRunning = true;
  grandTotalBytes = p.grand_total_bytes;
  grandTotalFiles = p.grand_total_files;

  // 1. Fully regenerate queue items
  queueItems = p.items.map(it => ({
    id: it.id,
    name: it.name,
    path: it.path,
    total_bytes: it.total_bytes,
    total_files: it.total_files,
    bytes_processed: 0,
    files_processed: 0,
    completed: false,
    success: null,
    isZip: p.is_zip,
  }));
  renderQueue();
  triggerAutoScroll(true);

  // 2. Pre-populate totals upfront on status bar and rings
  resetProgress(p.is_zip);
  const isBinary = currentSettings.UseBinaryUnits ?? true;
  txtCounter.textContent = `0/${p.total_items}`;
  if (grandTotalBytes > 0) {
    txtSummaryBytes.textContent = `0 B / ${formatBytes(grandTotalBytes, isBinary)}`;
  } else {
    txtSummaryBytes.textContent = '';
  }
  if (grandTotalFiles > 0) {
    txtSummaryFiles.textContent = `(0/${grandTotalFiles})`;
  } else {
    txtSummaryFiles.textContent = '';
  }
  panelSummary.style.display = 'flex';
});

listen('item-completed', (e) => {
  const p = e.payload;
  const item = queueItems.find(q => q.id === p.id);
  if (item) {
    item.completed = true;
    item.success = p.success;
    if (p.success) {
      if (item.total_bytes === 0) item.total_bytes = p.bytes;
      if (item.total_files === 0) item.total_files = p.files;
      item.bytes_processed = item.total_bytes;
      item.files_processed = item.total_files;
    }
    updateQueueItemElement(item);
    triggerAutoScroll(false);
  }
});

listen('op-progress', (e) => {
  const p = e.payload;
  const isBinary = currentSettings.UseBinaryUnits ?? true;

  // 1. Update the specific item in queue flyout
  const targetName = p.current_item_name || '';
  let qItem = queueItems.find(q =>
    q.name === targetName ||
    (targetName && (q.name.startsWith(targetName) || targetName.startsWith(q.name))) ||
    (targetName && q.path.includes(targetName))
  );
  if (!qItem && queueItems[p.item_index]) {
    qItem = queueItems[p.item_index];
  }
  if (!qItem && queueItems.length === 1) {
    qItem = queueItems[0];
  }

  if (qItem) {
    if (qItem.total_bytes === 0 && p.total_bytes > 0) qItem.total_bytes = p.total_bytes;
    if (qItem.total_files === 0 && p.total_files > 0) qItem.total_files = p.total_files;
    qItem.bytes_processed = p.bytes_processed;
    qItem.files_processed = p.files_processed;
    updateQueueItemElement(qItem);
    triggerAutoScroll(false);
  }

  // 2. Aggregate global progress across ALL items in the batch
  let processedBytesAll = 0;
  let processedFilesAll = 0;
  let completedCount = 0;

  for (const item of queueItems) {
    processedBytesAll += item.bytes_processed;
    processedFilesAll += item.files_processed;
    if (item.completed) {
      completedCount++;
    }
  }

  const effectiveTotalBytes = grandTotalBytes > 0 ? grandTotalBytes : (p.total_bytes > 0 ? p.total_bytes : 1);
  const effectiveTotalFiles = grandTotalFiles > 0 ? grandTotalFiles : (p.total_files > 0 ? p.total_files : 1);

  const byteRatio = Math.min(Math.max(processedBytesAll / effectiveTotalBytes, 0), 1);
  const fileRatio = Math.min(Math.max(processedFilesAll / effectiveTotalFiles, 0), 1);

  arcBytes.style.strokeDashoffset = OUTER_CIRCUMFERENCE * (1 - byteRatio);
  arcFiles.style.strokeDashoffset = INNER_CIRCUMFERENCE * (1 - fileRatio);

  // Progressive Status Updates showing total across all operations
  txtCounter.textContent = `${completedCount}/${queueItems.length}`;
  if (grandTotalBytes > 0) {
    txtSummaryBytes.textContent = `${formatBytes(processedBytesAll, isBinary)} / ${formatBytes(grandTotalBytes, isBinary)}`;
  } else {
    txtSummaryBytes.textContent = '';
  }
  if (grandTotalFiles > 0) {
    txtSummaryFiles.textContent = `(${processedFilesAll}/${grandTotalFiles})`;
  } else {
    txtSummaryFiles.textContent = '';
  }
  panelSummary.style.display = 'flex';
});

listen('op-completed', (e) => {
  isOperationRunning = false;
  const res = e.payload;
  const isBinary = currentSettings.UseBinaryUnits ?? true;

  if (res.success) {
    // Fill rings to 100%
    arcBytes.style.strokeDashoffset = '0';
    arcFiles.style.strokeDashoffset = '0';

    // Show checkmark badge
    progressArcsGroup.style.opacity = '0';
    badgeCompleted.style.opacity = '1';
    badgeCompleted.style.pointerEvents = 'auto';

    // Final summary text: e.g. "285/285  33.45 GiB (20113)"
    txtCounter.textContent = `${res.items_completed}/${res.total_items}`;
    txtSummaryBytes.textContent = formatBytes(res.total_bytes, isBinary);
    txtSummaryFiles.textContent = `(${res.total_files})`;
    panelSummary.style.display = 'flex';

    // Mark remaining queue items completed if not already marked
    queueItems.forEach(q => {
      if (!q.completed) {
        q.completed = true;
        if (q.success === null) q.success = true;
        if (q.total_bytes > 0) q.bytes_processed = q.total_bytes;
        if (q.total_files > 0) q.files_processed = q.total_files;
        updateQueueItemElement(q);
      }
    });

    // Auto-reset global rings after 3.5 seconds
    setTimeout(() => {
      if (!isOperationRunning) {
        badgeCompleted.style.opacity = '0';
        badgeCompleted.style.pointerEvents = 'none';
        progressArcsGroup.style.opacity = '1';
        arcBytes.style.strokeDashoffset = OUTER_CIRCUMFERENCE;
        arcFiles.style.strokeDashoffset = INNER_CIRCUMFERENCE;
      }
    }, 3500);
  } else {
    txtCounter.textContent = 'Error';
    console.error('Operation failed:', res.error_message);
  }
});

// Start
initSettings();
