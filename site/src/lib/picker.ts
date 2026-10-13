// The package picker's decision, from the data in src/data/packages.yaml (which the install
// guide's package table is also rendered from).

export interface PackageData {
  packages: {
    id: string;
    implementation: string;
    architectures: string[];
    glibc_min: string | null;
    binary_size: string;
    protocols: string[];
    features: string[];
  }[];
  features: Record<string, { label: string }>;
  protocols: Record<string, string>;
  distributions: { id: string; label: string; glibc: string | null }[];
  rust_glibc_comfort: string;
}

export interface Answers {
  architecture: string;
  distribution: string;
  protocols: string[];
  features: string[];
  smallFootprint: boolean;
}

export interface Recommendation {
  pick: 'tedge-dot-rs' | 'tedge-dot-c';
  /** Why this package, one sentence each. */
  reasons: string[];
  /** What the recommended package will not do for these answers. */
  caveats: string[];
  /** Another way to get there, when there is one. */
  alternative?: string;
}

function versionLess(a: string, b: string): boolean {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d < 0;
  }
  return false;
}

export function recommend(data: PackageData, a: Answers): Recommendation {
  const rs = data.packages.find((p) => p.id === 'tedge-dot-rs')!;
  const c = data.packages.find((p) => p.id === 'tedge-dot-c')!;
  const distro = data.distributions.find((d) => d.id === a.distribution);
  const needC: string[] = [];
  const needRs: string[] = [];

  if (a.architecture && !rs.architectures.includes(a.architecture)) {
    needC.push(
      `tedge-dot-rs is built for ${rs.architectures.join(' and ')} only; tedge-dot-c is also built for ${c.architectures
        .filter((x) => !rs.architectures.includes(x))
        .join(', ')}.`,
    );
  }
  if (distro?.glibc && versionLess(distro.glibc, data.rust_glibc_comfort)) {
    needC.push(
      `${distro.label} ships glibc ${distro.glibc}. tedge-dot-c is built against a glibc ${c.glibc_min} floor, so it runs there; the Rust package is not tested on distributions that old.`,
    );
  }
  const profibus = a.protocols.includes('profibus');
  if (profibus) {
    needC.push('PROFIBUS-DP ships in tedge-dot-c only: the Rust package is built without it.');
  }
  if (a.smallFootprint) {
    needC.push(`tedge-dot-c is about ${c.binary_size}, against ${rs.binary_size} for the Rust binary.`);
  }
  const rsOnly = a.features.filter((f) => rs.features.includes(f) && !c.features.includes(f));
  for (const f of rsOnly) {
    needRs.push(`${data.features[f]?.label ?? f} is supported by tedge-dot-rs only.`);
  }

  // Only PROFIBUS asks for C, and a Rust-only feature asks for Rust: build Rust with PROFIBUS.
  const onlyProfibus = needC.length === 1 && profibus;
  const sourceBuild =
    'Build tedge-dot-rs from source on Linux with `cargo build --manifest-path impl/rust/Cargo.toml --features profibus`.';

  if (needC.length && !(needRs.length && onlyProfibus)) {
    return {
      pick: 'tedge-dot-c',
      reasons: needC,
      caveats: needRs.map((r) => r.replace('is supported by tedge-dot-rs only.', 'is not available in tedge-dot-c.')),
      alternative: onlyProfibus ? `Or keep the Rust build: ${sourceBuild[0].toLowerCase()}${sourceBuild.slice(1)}` : undefined,
    };
  }
  if (needRs.length && onlyProfibus) {
    return {
      pick: 'tedge-dot-rs',
      reasons: [...needRs, 'PROFIBUS-DP is not in the released Rust package, but the Rust source has it.'],
      caveats: [],
      alternative: sourceBuild,
    };
  }
  return {
    pick: 'tedge-dot-rs',
    reasons: needRs.length
      ? needRs
      : ['It is the default: the richest protocol support, in one static binary with no shared-library dependencies.'],
    caveats: [],
  };
}
