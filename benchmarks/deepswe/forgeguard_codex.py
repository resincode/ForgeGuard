"""Pier Codex treatment that installs ForgeGuard before solving a task."""

from pier.agents.installed.codex import Codex
from pier.models.agent.install import InstallStep
from pier.models.agent.network import NetworkAllowlist

from release_tag import validated_release_tag


class ForgeGuardCodex(Codex):
    def __init__(self, *args, forgeguard_version="v0.14.0", **kwargs):
        self._forgeguard_version = validated_release_tag(forgeguard_version)
        super().__init__(*args, **kwargs)

    @staticmethod
    def name():
        return "codex-forgeguard"

    def network_allowlist(self):
        return NetworkAllowlist(
            domains=[
                *super().network_allowlist().domains,
                "github.com",
                "raw.githubusercontent.com",
                "release-assets.githubusercontent.com",
            ]
        )

    def install_spec(self):
        spec = super().install_spec()
        version = self._forgeguard_version
        spec.agent_name = self.name()
        spec.version = f"codex={spec.version or 'latest'},forgeguard={version}"
        spec.steps.append(
            InstallStep(
                user="root",
                env={
                    "FORGEGUARD_VERSION": version,
                    "FORGEGUARD_INSTALL_DIR": "/usr/local/bin",
                    "FORGEGUARD_PROFILE": "/dev/null",
                },
                run=(
                    "curl -fsSL --retry 3 "
                    f"https://raw.githubusercontent.com/suiflex/ForgeGuard/{version}/install.sh "
                    "| sh && forgeguard --version"
                ),
            )
        )
        return spec

    async def setup(self, environment):
        await super().setup(environment)
        await self.exec_as_agent(
            environment,
            command="""
set -eu
exclude_file="$(git rev-parse --git-path info/exclude)"
mkdir -p "$(dirname "$exclude_file")"
had_agents=0
[ -e AGENTS.md ] && had_agents=1
gitignore_backup=""
if [ -f .gitignore ]; then
  gitignore_backup="$(mktemp)"
  cp .gitignore "$gitignore_backup"
fi
restore_gitignore() {
  if [ -n "$gitignore_backup" ]; then
    cp "$gitignore_backup" .gitignore
    rm -f "$gitignore_backup"
  fi
}
trap restore_gitignore EXIT
forgeguard init --agent codex
restore_gitignore
trap - EXIT
printf '\n# ForgeGuard DeepSWE treatment\n.forgeguard/\n.codex/\n.agents/\n' >> "$exclude_file"
if [ "$had_agents" -eq 0 ]; then
  printf 'AGENTS.md\n' >> "$exclude_file"
fi
""",
            timeout_sec=120,
        )
