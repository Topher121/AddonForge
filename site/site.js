// Addon directory: filter rows by search text and type. Runs entirely in the
// page; nothing is sent anywhere.
(function () {
  var q = document.getElementById("q");
  var cat = document.getElementById("cat");
  var count = document.getElementById("count");
  var rows = document.querySelectorAll("#rows tr");
  if (!q || !cat || !rows.length) return;
  function apply() {
    var words = q.value.toLowerCase().split(/\s+/).filter(Boolean);
    var want = cat.value;
    var shown = 0;
    rows.forEach(function (tr) {
      var text = tr.getAttribute("data-text") || "";
      var ok = (!want || tr.getAttribute("data-cat") === want) &&
        words.every(function (w) { return text.indexOf(w) !== -1; });
      tr.hidden = !ok;
      if (ok) shown++;
    });
    count.textContent = shown === rows.length ? "" : shown + " of " + rows.length;
  }
  q.addEventListener("input", apply);
  cat.addEventListener("change", apply);
  apply();
})();
