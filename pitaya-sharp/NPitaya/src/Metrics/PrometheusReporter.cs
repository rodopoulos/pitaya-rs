using System.Collections.Generic;
using System.Security.Authentication.ExtendedProtection;
using Microsoft.Extensions.DependencyInjection;
using NPitaya.Models;
using Prometheus;
using Prometheus.DotNetRuntime;
using Prometheus.SystemMetrics;

namespace NPitaya.Metrics
{
    public class PrometheusReporter
    {
        const string LabelSeparator = "_";
        static readonly string[] NoLabels = new string[] {};

        readonly string _host;
        readonly int _port;
        readonly string _namespace;
        readonly MetricServer _server;
        readonly DotNetRuntimeStatsBuilder.Builder _dotnetCollector;
        readonly IServiceCollection _systemMetrics;

        readonly Dictionary<string, Counter> _counters;
        readonly Dictionary<string, Gauge> _gauges;
        readonly Dictionary<string, Histogram> _histograms;

        internal PrometheusReporter(MetricsConfiguration config) : this(
            config.Host,
            config.Port,
            config.Namespace,
            config.CustomMetrics
        ){}

        private PrometheusReporter(string host, string port, string @namespace, CustomMetrics customMetrics)
        {
            _namespace = @namespace;
            _host = host;
            _port = int.Parse(port);
            _server = new MetricServer(hostname: _host, port: _port);
            _counters = new Dictionary<string, Counter>();
            _gauges = new Dictionary<string, Gauge>();
            _histograms = new Dictionary<string, Histogram>();
            _dotnetCollector = DotNetRuntimeStatsBuilder
                .Customize()
                .WithContentionStats()
                .WithThreadPoolSchedulingStats()
                .WithThreadPoolStats()
                .WithGcStats()
                .WithExceptionStats();
            _systemMetrics = new ServiceCollection();
            ImportCustomMetrics(customMetrics);
        }

        internal void Start()
        {
            Logger.Info("Starting Prometheus metrics server at {0}:{1}", _host, _port);
            _dotnetCollector?.StartCollecting();
            _systemMetrics.AddSystemMetrics();
            _server.Start();
        }

        internal void RegisterCounter(string name, string help = null, string[] labels = null)
        {
            var key = BuildKey(name);
            Logger.Debug($"Registering counter metric {key}");
            var counter = Prometheus.Metrics.CreateCounter(key, help ?? "", labels ?? NoLabels);
            _counters.Add(key, counter);
        }

        internal void RegisterGauge(string name, string help = null, string[] labels = null)
        {
            var key = BuildKey(name);
            Logger.Debug($"Registering gauge metric {key}");
            var gauge = Prometheus.Metrics.CreateGauge(key, help ?? "", labels ?? NoLabels);
            _gauges.Add(key, gauge);
        }

        internal void RegisterHistogram(string name, string help = null, string[] labels = null)
        {
            var key = BuildKey(name);
            Logger.Debug($"Registering histogram metric {key}");
            var histogram = Prometheus.Metrics.CreateHistogram(key, help ?? "", labels ?? NoLabels);
            _histograms.Add(key, histogram);
        }

        internal void IncCounter(string name, string[] labels = null)
        {
            var key = BuildKey(name);
            var counter = _counters[key];
            Logger.Debug($"Incrementing counter {key}");
            counter.WithLabels(labels ?? NoLabels).Inc();
        }

        internal void SetGauge(string name, double value, string[] labels = null)
        {
            var key = BuildKey(name);
            var gauge = _gauges[key];
            Logger.Debug($"Setting gauge {key} with value {value}");
            gauge.WithLabels(labels ?? NoLabels).Set(value);
        }

        internal void ObserveHistogram(string name, double value, string[] labels = null)
        {
            var key = BuildKey(name);
            var histogram = _histograms[key];
            Logger.Debug($"Observing histogram {key} with value {value}");
            histogram.WithLabels(labels ?? NoLabels).Observe(value);
        }

        string BuildKey(string suffix)
        {
            return $"{_namespace}{LabelSeparator}{suffix}";
        }

        void ImportCustomMetrics(CustomMetrics metrics)
        {
            if (metrics == null) return;

            foreach (var metric in metrics.Counters)
            {
                RegisterCounter(metric.Name, metric.Help, metric.Labels);
            }

            foreach (var metric in metrics.Gauges)
            {
                RegisterGauge(metric.Name, metric.Help, metric.Labels);
            }

            foreach (var metric in metrics.Histograms)
            {
                RegisterHistogram(metric.Name, metric.Help, metric.Labels);
            }
        }
    }
}