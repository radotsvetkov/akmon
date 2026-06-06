class Akmon < Formula
  desc "Tamper-evident evidence and verification layer for AI agents"
  homepage "https://github.com/radotsvetkov/akmon"
  version "2.2.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/akmon-darwin-arm64"
      sha256 "ee6e77675202e09c36136fc31bd6e7eb89a1ff6712a3eb0de393cfad499b483a"
    end
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/akmon-darwin-x86_64"
      sha256 "5433f7d7180c66b88470fbb02537d1b667185cf992f16ecdab5a81d38b8132e3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/radotsvetkov/akmon/releases/download/v2.2.0/akmon-linux-x86_64"
      sha256 "5ac9cce0df55d7820162214a47d742c93b8145a05c7f6ab8c5158c862c5850ca"
    end
  end

  def install
    bin.install Dir["akmon-*"].first => "akmon"
  end

  test do
    assert_match "2.2.0", shell_output("#{bin}/akmon --version")
  end
end
