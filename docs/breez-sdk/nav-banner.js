// Inject a link to the API Migration Guide in the mdBook top bar
(function () {
  var bar = document.querySelector('.right-buttons');
  if (!bar) return;
  var link = document.createElement('a');
  link.href = '../api-migration-guide.html';
  link.title = 'API Migration Guide';
  link.textContent = 'Migration Guide';
  link.style.cssText =
    'display:inline-flex;align-items:center;padding:4px 12px;margin-left:8px;' +
    'font-size:13px;font-weight:600;color:#fff;background:#7c6bf5;border-radius:6px;' +
    'text-decoration:none;transition:opacity .15s;';
  link.onmouseover = function () { this.style.opacity = '0.85'; };
  link.onmouseout = function () { this.style.opacity = '1'; };
  bar.prepend(link);
})();
