// Install tabs, the copy button, and the recall numbers from the latest
// benchmark run (docs/benchmarks.json, written by the bench workflow).
(function () {
  var text = document.getElementById("cmd-text");
  var copy = document.getElementById("copy");
  var tabs = document.querySelectorAll(".tabs button");
  tabs.forEach(function (tab) {
    tab.addEventListener("click", function () {
      tabs.forEach(function (t) { t.setAttribute("aria-selected", String(t === tab)); });
      text.textContent = tab.dataset.cmd;
    });
  });
  copy.addEventListener("click", function () {
    var done = function () {
      copy.textContent = "Copied";
      setTimeout(function () { copy.textContent = "Copy"; }, 1400);
    };
    if (navigator.clipboard) {
      navigator.clipboard.writeText(text.textContent).then(done, select);
    } else {
      select();
    }
  });
  function select() {
    var range = document.createRange();
    range.selectNodeContents(text);
    var sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  var stats = document.getElementById("stats");
  var note = document.getElementById("stats-note");
  fetch("docs/benchmarks.json")
    .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
    .then(function (data) {
      var repos = (data.repos || []).filter(function (r) { return r.gate_met; });
      if (!repos.length) return;
      stats.textContent = "";
      repos.forEach(function (r) {
        var cell = document.createElement("div");
        cell.className = "stat";
        cell.innerHTML = '<span class="n"></span><span class="repo"></span><span class="l"></span>';
        cell.querySelector(".n").textContent = (r.recall * 100).toFixed(1) + "%";
        cell.querySelector(".repo").textContent = r.repo;
        cell.querySelector(".l").textContent = r.judged + " failures, " + r.from + " to " + r.until;
        stats.appendChild(cell);
      });
      note.innerHTML = 'Planned with Fairlead <span></span>. Flaky and quarantined failures are left out; the <a href="docs/benchmarks.html">benchmarks page</a> has every miss and the raw figures.';
      note.querySelector("span").textContent = data.fairlead;
    })
    .catch(function () { /* the fallback link stays */ });
})();
