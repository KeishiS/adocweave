import { execFileSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { relative, resolve, sep } from "node:path";

function listAsciiDocFiles(repositoryRoot, directory) {
  const files = [];

  function visit(currentDirectory) {
    for (const entry of readdirSync(currentDirectory, { withFileTypes: true })) {
      const absolute = resolve(currentDirectory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile() && entry.name.endsWith(".adoc")) {
        files.push(relative(repositoryRoot, absolute).split(sep).join("/"));
      }
    }
  }

  visit(resolve(repositoryRoot, directory));
  return files;
}

export function listRepositoryAsciiDocFiles(repositoryRoot) {
  const tracked = execFileSync("git", ["ls-files", "-z", "*.adoc"], {
    cwd: repositoryRoot,
    encoding: "utf8"
  })
    .split("\0")
    .filter(Boolean);
  const docs = listAsciiDocFiles(repositoryRoot, "docs");

  return Object.freeze([...new Set([...tracked, ...docs])].sort());
}
