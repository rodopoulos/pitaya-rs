using System;

namespace NPitaya.Metrics
{
    class MetricsReporter
    {
        readonly bool _isEnabled;
        readonly PrometheusReporter _prometheusServer;
        readonly PitayaReporter _pitayaMetrics ;

        internal MetricsReporter(MetricsConfiguration config)
        {
            _isEnabled = config.IsEnabled;
            if (!_isEnabled)
            {
                return;
            }
            _prometheusServer = new PrometheusReporter(config);
            _pitayaMetrics = new PitayaReporter(_prometheusServer);
        }

        internal void Start()
        {
            if (!_isEnabled)
            {
                return;
            }

            _prometheusServer.Start();
        }

        internal IntPtr GetPitayaPrt()
        {
            return !_isEnabled ? IntPtr.Zero : _pitayaMetrics.Ptr;
        }
    }
}