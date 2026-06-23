async function loadLatestRelease() {
  const summary = document.getElementById("release-summary");
  const list = document.getElementById("download-list");
  const releaseLink = document.getElementById("release-link");

  try {
    const response = await fetch("./releases/latest.json", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`release metadata returned ${response.status}`);
    }

    const release = await response.json();
    if (!release.tag_name || !Array.isArray(release.assets)) {
      throw new Error("latest release has no downloadable assets yet");
    }

    summary.textContent = `${release.name || release.tag_name} was published with ${release.assets.length} release assets.`;
    releaseLink.href = release.html_url;
    releaseLink.hidden = false;

    const debAssets = release.assets.filter((asset) => asset.name.endsWith(".deb"));
    const checksumAssets = release.assets.filter((asset) =>
      asset.name.toLowerCase().includes("sha256"),
    );
    const visibleAssets = debAssets.length > 0 ? debAssets : release.assets;

    list.replaceChildren(
      ...visibleAssets.map((asset) => downloadItem(asset)),
      ...checksumAssets.map((asset) => downloadItem(asset)),
    );
  } catch (error) {
    summary.textContent =
      "No GitHub Release metadata is available yet. Publish a v* tag and this section will update automatically.";
    list.replaceChildren();
  }
}

function downloadItem(asset) {
  const link = document.createElement("a");
  link.className = "download-item";
  link.href = asset.browser_download_url;
  link.innerHTML = `<strong>${asset.name}</strong><span>${formatSize(asset.size)}</span>`;
  return link;
}

function formatSize(size) {
  if (!Number.isFinite(size) || size <= 0) {
    return "";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = size;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(value >= 10 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

loadLatestRelease();
