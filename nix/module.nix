{ config, lib, pkgs, ... }:

let
  cfg = config.services.starla;
  settingsFormat = pkgs.formats.toml { };

  configFile = settingsFormat.generate "config.toml" {
    probe.log_level = cfg.logLevel;
    network.rxtxrpt = cfg.reportInterfaceStats;
    network.status_socket = cfg.statusSocket;
    controller = {
      registration_servers = cfg.controller.registrationServers;
      ssh_timeout = cfg.controller.sshTimeout;
      keepalive_interval = cfg.controller.keepaliveInterval;
    };
    storage.max_queue_size = cfg.storage.maxQueueSize;
    metrics = {
      enabled = cfg.metrics.enable;
      listen_addr = cfg.metrics.listenAddr;
    };
    logging.format = cfg.logging.format;
  };

  # Options that never did anything. Still accepted, so that a configuration
  # setting one evaluates, but with a warning, and not written to config.toml.
  ignoredOption = lib.mkOption {
    type = lib.types.nullOr lib.types.anything;
    default = null;
    visible = false;
    description = "Ignored: Starla never read it. Setting it only warns.";
  };

  ignored = lib.filterAttrs (_: o: o.value != null) {
    "storage.retentionDays" = {
      value = cfg.storage.retentionDays;
      why = "Starla keeps no results on disk to expire";
    };
    "logging.output" = {
      value = cfg.logging.output;
      why = "the service logs to stdout, which systemd sends to the journal";
    };
    "logging.maxFileSizeMB" = {
      value = cfg.logging.maxFileSizeMB;
      why = "the service writes no log files; the journal has its own limits";
    };
    "logging.maxFiles" = {
      value = cfg.logging.maxFiles;
      why = "the service writes no log files; the journal has its own limits";
    };
  };
in
{
  options.services.starla = {
    enable = lib.mkEnableOption "Starla RIPE Atlas software probe";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The starla package to use.";
    };

    logLevel = lib.mkOption {
      type = lib.types.enum [ "trace" "debug" "info" "warn" "error" ];
      default = "info";
      description = "Log verbosity level.";
    };

    sshKeyFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to a file containing the SSH private key (OpenSSH PEM format).
        When set, starla reads the key from this file via the STARLA_SSH_KEY
        environment variable instead of generating one in the state directory.
        Use this to inject keys from secret managers like agenix or sops-nix.
      '';
    };

    reportInterfaceStats = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Report network interface traffic statistics (rx/tx bytes, packets, errors) to the RIPE Atlas controller.";
    };

    statusSocket = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable the Unix domain socket for tray app communication. Only needed on desktop systems.";
    };

    controller = {
      registrationServers = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [
          "reg-05.prod.atlas.ripe.net:443"
          "reg-06.prod.atlas.ripe.net:443"
        ];
        description = "RIPE Atlas registration servers (host:port).";
      };

      sshTimeout = lib.mkOption {
        type = lib.types.ints.positive;
        default = 30;
        description = "SSH connection timeout in seconds.";
      };

      keepaliveInterval = lib.mkOption {
        type = lib.types.ints.positive;
        default = 60;
        description = "SSH keepalive interval in seconds.";
      };
    };

    storage = {
      maxQueueSize = lib.mkOption {
        type = lib.types.ints.positive;
        default = 10000;
        description = "Maximum number of results to hold in the upload queue.";
      };

      retentionDays = ignoredOption;
    };

    metrics = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable Prometheus metrics export.";
      };

      listenAddr = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1:9695";
        description = "Metrics server listen address.";
      };
    };

    logging = {
      format = lib.mkOption {
        type = lib.types.enum [ "json" "text" ];
        default = "json";
        description = "Log output format.";
      };

      output = ignoredOption;
      maxFileSizeMB = ignoredOption;
      maxFiles = ignoredOption;
    };
  };

  config = lib.mkIf cfg.enable {
    warnings = lib.mapAttrsToList
      (name: o: "services.starla.${name} does nothing and is ignored: ${o.why}.")
      ignored;

    systemd.services.starla = {
      description = "Starla RIPE Atlas Software Probe";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "simple";
        Restart = "on-failure";
        RestartSec = 10;

        # Scheduler bursts past the 1024 default under load.
        LimitNOFILE = 4096;

        DynamicUser = true;

        # Directories: systemd sets CONFIGURATION_DIRECTORY, STATE_DIRECTORY,
        # RUNTIME_DIRECTORY which starla's path resolution picks up automatically
        ConfigurationDirectory = "starla";
        ConfigurationDirectoryMode = "0755";
        StateDirectory = "starla";
        StateDirectoryMode = "0750";
        RuntimeDirectory = "starla";
        RuntimeDirectoryMode = "0750";
        # Files Starla creates (probe_id, paused_until, the status socket)
        # are its own alone; see starla.service.
        UMask = "0077";

        # Raw sockets for ping and traceroute
        AmbientCapabilities = [ "CAP_NET_RAW" ];
        CapabilityBoundingSet = [ "CAP_NET_RAW" ];

        # Hardening
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        # Not ProcSubset = "pid": host telemetry reads /proc/uptime,
        # /proc/buddyinfo, /proc/net and /proc/sys.
        ProtectProc = "invisible";
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        SystemCallArchitectures = "native";
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" "AF_NETLINK" ];
        SystemCallFilter = [ "@system-service" "~@privileged" ];
        SystemCallErrorNumber = "EPERM";
        ExecStart = "${cfg.package}/bin/starla --config %E/starla/config.toml";
      } // lib.optionalAttrs (cfg.sshKeyFile != null) {
        LoadCredential = [ "ssh-key:${cfg.sshKeyFile}" ];
      };
    };

    # Install the config file into the configuration directory
    environment.etc."starla/config.toml".source = configFile;
  };

  meta.maintainers = [ ];
}
