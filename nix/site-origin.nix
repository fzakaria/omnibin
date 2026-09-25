# Where the site is served from.
#
# site/index.html states the same origin in its canonical link and
# site/js/config.js in SITE_ORIGIN; a sitemap has to name absolute URLs and
# the rendered docs have to name them in their own canonical links, so this is
# the third statement of it and the only one the Nix side makes.
{
  siteOrigin = "https://omnibin.dev";
}
