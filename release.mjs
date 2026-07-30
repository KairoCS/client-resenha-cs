// release.mjs — publica uma versão nova do Resenha Client.
//
//   node release.mjs 0.3.0
//
// 1. sobe a versão nos três manifests (Cargo.toml, tauri.conf.json, package.json);
// 2. roda `tauri build` (gera o instalador NSIS);
// 3. copia o instalador pro site (public/client/ResenhaClient-setup.exe).
//
// Depois disso falta só: subir CLIENT_LATEST no backend (src/version.js ou a
// env CLIENT_LATEST_VERSION no Render) e fazer deploy dos dois. Enquanto o
// backend não subir, quem já tem a versão anterior continua funcionando —
// é o backend que decide quando a atualização vira obrigatória.
import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const raiz = path.dirname(fileURLToPath(import.meta.url));
const versao = process.argv[2];

if (!/^\d+\.\d+\.\d+$/.test(versao ?? '')) {
  console.error('uso: node release.mjs <versao>   (ex.: node release.mjs 0.3.0)');
  process.exit(1);
}

const troca = (arquivo, de, para) => {
  const caminho = path.join(raiz, arquivo);
  const antes = fs.readFileSync(caminho, 'utf8');
  const depois = antes.replace(de, para);
  if (antes === depois) throw new Error(`não achei a versão em ${arquivo}`);
  fs.writeFileSync(caminho, depois);
  console.log(`✓ ${arquivo}`);
};

troca('src-tauri/Cargo.toml', /^version = "\d+\.\d+\.\d+"/m, `version = "${versao}"`);
troca('src-tauri/tauri.conf.json', /"version": "\d+\.\d+\.\d+"/, `"version": "${versao}"`);
troca('package.json', /"version": "\d+\.\d+\.\d+"/, `"version": "${versao}"`);

console.log(`\nbuildando v${versao} (leva alguns minutos)…`);
execSync('npx tauri build', { cwd: raiz, stdio: 'inherit' });

const origem = path.join(
  raiz, 'src-tauri/target/release/bundle/nsis', `Resenha Client_${versao}_x64-setup.exe`
);
const destinoDir = path.join(raiz, '../resenha-cs-next/public/client');
fs.mkdirSync(destinoDir, { recursive: true });
const destino = path.join(destinoDir, 'ResenhaClient-setup.exe');
fs.copyFileSync(origem, destino);

const mb = (fs.statSync(destino).size / 1024 / 1024).toFixed(2);
console.log(`\n✓ instalador v${versao} copiado pro site (${mb} MB)`);
console.log(`\nFalta: CLIENT_LATEST = '${versao}' no backend (src/version.js) + deploy dos dois.`);
