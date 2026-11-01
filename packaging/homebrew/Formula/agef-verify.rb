class AgefVerify < Formula
  desc "Standalone offline verifier for AGEF agent-evidence bundles"
  homepage "https://github.com/radotsvetkov/akmon"
  version "3.0.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/agef-verify-darwin-arm64"
      sha256 "731cd436826762d59ff7207d950e3939e859dd0e08f5348927bc4a5f8c5f3102"
    end
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/agef-verify-darwin-x86_64"
      sha256 "369091cf888cad96cd745681412621adcdca4c0e434bfb820331a8559f658218"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v3.0.0/agef-verify-linux-x86_64"
      sha256 "0f9b5bc78101ab72d4636f2bf087d5aa236b17b3ba2e78fe56f869b6e9f613ab"
    end
  end

  def install
    bin.install Dir["agef-verify-*"].first => "agef-verify"
  end

  test do
    assert_match "agef-verify", shell_output("#{bin}/agef-verify --help 2>&1")
  end
end
