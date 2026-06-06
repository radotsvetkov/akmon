class AgefVerify < Formula
  desc "Standalone offline verifier for AGEF agent-evidence bundles"
  homepage "https://github.com/radotsvetkov/akmon"
  version "2.2.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/agef-verify-darwin-arm64"
      sha256 "fab647ce223916e895abc0c96825d7b36bda3956a4ebd4db25acd1a2a365a0ac"
    end
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/agef-verify-darwin-x86_64"
      sha256 "aba425aa3e7b1971215a553f5d7de8622a7ece5dd078e07ca784ce0cf700e5c1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/agef-verify-linux-x86_64"
      sha256 "8ebf214e6061f77260c3ea536cfb8429c79601e296591986d1649e55c9568593"
    end
  end

  def install
    bin.install Dir["agef-verify-*"].first => "agef-verify"
  end

  test do
    assert_match "agef-verify", shell_output("#{bin}/agef-verify --help 2>&1")
  end
end
