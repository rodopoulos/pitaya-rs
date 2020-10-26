using System;
using System.Runtime.InteropServices;
using NPitaya.Models;

namespace NPitaya.Metrics
{
    public class PitayaReporter
    {
        const string LabelSeparator = "_";
        const string PitayaSubsystem = "pitaya";

        public IntPtr Ptr { get; }

        public PitayaReporter(PrometheusReporter prometheusReporter)
        {
            var handle = GCHandle.Alloc(prometheusReporter, GCHandleType.Normal);
            var reporterPtr = GCHandle.ToIntPtr(handle);
            Ptr = PitayaCluster.pitaya_metrics_reporter_new(
                RegisterCounterFn,
                RegisterHistogramFn,
                RegisterGaugeFn,
                IncCounterFn,
                ObserveHistFn,
                SetGaugeFn,
                AddGaugeFn,
                reporterPtr);
        }

        ~PitayaReporter()
        {
            PitayaCluster.pitaya_metrics_reporter_drop(Ptr);
        }

        static void RegisterCounterFn(IntPtr prometheusPtr, MetricsOpts opts)
        {
            // TODO (felipe.rodopoulos): prometheus-net does not support subsystem label yet. We'll use a hardcoded one.
            var name = Marshal.PtrToStringAnsi(opts.Name);
            if (string.IsNullOrEmpty(name))
            {
                Logger.Warn("Tried to register a counter with an empty name");
                return;
            }
            var help = Marshal.PtrToStringAnsi(opts.Help) ?? string.Empty;
            var labels = ReadLabels(ref opts.VariableLabels, opts.VariableLabelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            var key = BuildKey(name);
            prometheus?.RegisterCounter(key, help, labels);
        }

        static void RegisterHistogramFn(IntPtr prometheusPtr, MetricsOpts opts)
        {
            // TODO (felipe.rodopoulos): prometheus-net does not support subsystem label yet. We'll use a hardcoded one.
            var name = Marshal.PtrToStringAnsi(opts.Name);
            if (string.IsNullOrEmpty(name))
            {
                Logger.Warn("Tried to register an histogram with an empty name");
                return;
            }
            var help = Marshal.PtrToStringAnsi(opts.Help) ?? string.Empty;
            var labels = ReadLabels(ref opts.VariableLabels, opts.VariableLabelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            var key = BuildKey(name);
            prometheus?.RegisterHistogram(key, help, labels);
        }

        static void RegisterGaugeFn(IntPtr prometheusPtr, MetricsOpts opts)
        {
            // TODO (felipe.rodopoulos): prometheus-net does not support subsystem label yet. We'll use a hardcoded one.
            var name = Marshal.PtrToStringAnsi(opts.Name);
            if (string.IsNullOrEmpty(name))
            {
                Logger.Warn("Tried to register a gaugee with an empty name");
                return;
            }
            var help = Marshal.PtrToStringAnsi(opts.Help) ?? string.Empty;
            var labels = ReadLabels(ref opts.VariableLabels, opts.VariableLabelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            var key = BuildKey(name);
            prometheus?.RegisterGauge(key, help, labels);
        }

        static void IncCounterFn(IntPtr prometheusPtr, IntPtr name, ref IntPtr labels, UInt32 labelsCount)
        {
            string nameStr = Marshal.PtrToStringAnsi(name) ?? string.Empty;
            if (string.IsNullOrEmpty(nameStr))
            {
                Logger.Warn("Tried to increment a counter with an empty name");
                return;
            }
            var labelsArr = ReadLabels(ref labels, labelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            prometheus?.IncCounter(nameStr, labelsArr);
        }

        static void ObserveHistFn(IntPtr prometheusPtr, IntPtr name, double value, ref IntPtr labels, UInt32 labelsCount)
        {
            string nameStr = Marshal.PtrToStringAnsi(name) ?? string.Empty;
            if (string.IsNullOrEmpty(nameStr))
            {
                Logger.Warn("Tried to observe an histogram with an empty name");
                return;
            }
            var key = BuildKey(nameStr);
            var labelsArr = ReadLabels(ref labels, labelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            prometheus?.ObserveHistogram(key, value, labelsArr);
        }

        static void SetGaugeFn(IntPtr prometheusPtr, IntPtr name, double value, ref IntPtr labels, UInt32 labelsCount)
        {
            string nameStr = Marshal.PtrToStringAnsi(name) ?? string.Empty;
            if (string.IsNullOrEmpty(nameStr))
            {
                Logger.Warn("Tried to set a gauge with an empty name");
                return;
            }
            var labelsArr = ReadLabels(ref labels, labelsCount);
            var prometheus = RetrievePrometheus(prometheusPtr);
            prometheus?.SetGauge(nameStr, value, labelsArr);
        }

        static void AddGaugeFn(IntPtr prometheusPtr, IntPtr name, double value, ref IntPtr labels, UInt32 labelsCount)
        {
            string nameStr = Marshal.PtrToStringAnsi(name) ?? string.Empty;
            Logger.Warn($"Adding gauge {nameStr} with val {value}. This method should not be used.");
        }

        private static PrometheusReporter? RetrievePrometheus(IntPtr ptr)
        {
            var handle = GCHandle.FromIntPtr(ptr);
            return handle.Target as PrometheusReporter;
        }

        private static string BuildKey(string suffix)
        {
            return $"{PitayaSubsystem}{LabelSeparator}{suffix}";
        }
    }
}