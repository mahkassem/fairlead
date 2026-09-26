// The logo in the title bar goes to the site's front page, and wide screens get
// an on-this-page outline of the chapter's sections.
(function () {
  var title = document.querySelector(".menu-title");
  if (title) {
    title.setAttribute("role", "link");
    title.setAttribute("title", "Fairlead home");
    title.addEventListener("click", function () { window.location.href = "../"; });
  }

  var content = document.querySelector("main");
  if (!content) return;
  var heads = content.querySelectorAll("h2[id], h3[id]");
  if (heads.length < 2) return;
  var nav = document.createElement("nav");
  nav.className = "fl-outline";
  nav.setAttribute("aria-label", "On this page");
  var label = document.createElement("b");
  label.textContent = "On this page";
  nav.appendChild(label);
  var links = [];
  heads.forEach(function (h) {
    var a = document.createElement("a");
    a.href = "#" + h.id;
    a.textContent = h.textContent;
    if (h.tagName === "H3") a.className = "sub";
    nav.appendChild(a);
    links.push([h, a]);
  });
  document.body.appendChild(nav);
  document.body.classList.add("fl-outlined");

  function mark() {
    var current = links[0];
    links.forEach(function (pair) {
      if (pair[0].getBoundingClientRect().top < 120) current = pair;
    });
    links.forEach(function (pair) { pair[1].classList.toggle("on", pair === current); });
  }
  window.addEventListener("scroll", mark, { passive: true });
  mark();
})();
