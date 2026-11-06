// Signs build/latest.json for a release. The private key stays outside the
// repo: set ADDONFORGE_RELEASE_KEY to the path of release-private.pem.
//   node scripts/sign-release.js build/latest.json   -> writes build/latest.json.sig
//   node scripts/sign-release.js --verify build/latest.json
//   node scripts/sign-release.js --text catalog/forever.json   (the catalogue)
// The signature is base64 Ed25519 over the exact bytes of the file. With --text,
// over the bytes with every carriage return removed, so a CRLF checkout signs the same as
// GitHub's LF copy (src-tauri/src/catalog.rs signed_bytes does the same).
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const args = process.argv.slice(2);
const verifyOnly = args.includes("--verify");
const textMode = args.includes("--text");
const manifest = args.find((a) => !a.startsWith("--"));
const keyPath = process.env.ADDONFORGE_RELEASE_KEY;
if (!manifest || !keyPath) {
  console.error("usage: ADDONFORGE_RELEASE_KEY=<path> node scripts/sign-release.js [--verify] [--text] <file>");
  process.exit(2);
}

const raw = fs.readFileSync(manifest);
const data = textMode ? Buffer.from(raw.filter((b) => b !== 13)) : raw;
const privateKey = crypto.createPrivateKey(fs.readFileSync(keyPath));
const publicKey = crypto.createPublicKey(privateKey);

// The app only trusts the key built into src-tauri/src/signing.rs. Refuse to
// sign with any other key, or every installed copy would reject the update.
const rawKey = publicKey.export({ type: "spki", format: "der" }).subarray(-32);
const signing = fs.readFileSync(path.join(__dirname, "..", "src-tauri", "src", "signing.rs"), "utf8");
const block = signing.slice(signing.indexOf("RELEASE_PUBLIC_KEY: Option"));
const embedded = [...block.slice(0, block.indexOf("]);")).matchAll(/0x([0-9a-f]{2})/gi)].map((m) => parseInt(m[1], 16));
if (embedded.length !== 32 || !rawKey.equals(Buffer.from(embedded))) {
  console.error("private key does not match the public key built into the app (signing.rs). Not signing.");
  process.exit(1);
}

if (verifyOnly) {
  const sig = Buffer.from(fs.readFileSync(manifest + ".sig", "utf8").trim(), "base64");
  const ok = crypto.verify(null, data, publicKey, sig);
  console.log(ok ? "signature OK" : "signature DOES NOT MATCH");
  process.exit(ok ? 0 : 1);
}

const sig = crypto.sign(null, data, privateKey).toString("base64");
fs.writeFileSync(manifest + ".sig", sig + "\n");
console.log("wrote " + manifest + ".sig");
