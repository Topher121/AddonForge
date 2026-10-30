// Signs build/latest.json for a release. The private key stays outside the
// repo: set ADDONFORGE_RELEASE_KEY to the path of release-private.pem.
//   node scripts/sign-release.js build/latest.json   -> writes build/latest.json.sig
//   node scripts/sign-release.js --verify build/latest.json
// The signature is base64 Ed25519 over the exact bytes of the file.
const fs = require("fs");
const crypto = require("crypto");

const args = process.argv.slice(2);
const verifyOnly = args[0] === "--verify";
const manifest = args[verifyOnly ? 1 : 0];
const keyPath = process.env.ADDONFORGE_RELEASE_KEY;
if (!manifest || !keyPath) {
  console.error("usage: ADDONFORGE_RELEASE_KEY=<path> node scripts/sign-release.js [--verify] <latest.json>");
  process.exit(2);
}

const data = fs.readFileSync(manifest);
const privateKey = crypto.createPrivateKey(fs.readFileSync(keyPath));
const publicKey = crypto.createPublicKey(privateKey);

if (verifyOnly) {
  const sig = Buffer.from(fs.readFileSync(manifest + ".sig", "utf8").trim(), "base64");
  const ok = crypto.verify(null, data, publicKey, sig);
  console.log(ok ? "signature OK" : "signature DOES NOT MATCH");
  process.exit(ok ? 0 : 1);
}

const sig = crypto.sign(null, data, privateKey).toString("base64");
fs.writeFileSync(manifest + ".sig", sig + "\n");
console.log("wrote " + manifest + ".sig");
